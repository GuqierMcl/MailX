use crate::{
    AppError, ErrorCode,
    accounts::{model::*, types::*},
};
use fs2::FileExt;
use rusqlite::{Connection, OptionalExtension, params};
use std::{
    fs::{File, OpenOptions},
    path::Path,
    time::Duration,
};

pub(crate) mod bodies;
mod folders;
pub(crate) mod image_cache;
pub(crate) mod messages;
mod migrations;

pub(crate) struct Storage {
    pub db: Connection,
    pub(crate) blobs: bodies::BlobStore,
    _lock: File,
}

pub(crate) fn encode<T: serde::Serialize>(value: &T) -> Result<String, AppError> {
    serde_json::to_string(value).map_err(|_| AppError::new(ErrorCode::Internal))
}
fn decode<T: serde::de::DeserializeOwned>(value: &str) -> Result<T, AppError> {
    serde_json::from_str(value).map_err(|_| AppError::new(ErrorCode::StorageIncompatible))
}

impl Storage {
    pub fn open(directory: &Path) -> Result<Self, AppError> {
        std::fs::create_dir_all(directory)
            .map_err(|_| AppError::new(ErrorCode::StorageUnavailable))?;
        let lock = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(directory.join("mailx.lock"))
            .map_err(|_| AppError::new(ErrorCode::StorageUnavailable))?;
        FileExt::try_lock_exclusive(&lock).map_err(|error| {
            // Contention is EWOULDBLOCK on Unix but ERROR_LOCK_VIOLATION on
            // Windows, which has no matching ErrorKind; compare the OS code.
            let contended = error.raw_os_error().is_some()
                && error.raw_os_error() == fs2::lock_contended_error().raw_os_error();
            AppError::new(if contended {
                ErrorCode::AlreadyOpen
            } else {
                ErrorCode::StorageUnavailable
            })
        })?;
        let mut db = Connection::open(directory.join("mailx.sqlite3"))?;
        db.busy_timeout(Duration::from_secs(5))?;
        db.pragma_update(None, "foreign_keys", true)?;
        migrations::apply(&mut db, migrations::MIGRATIONS)?;
        let blobs = bodies::BlobStore::open(directory)?;
        let mut storage = Self {
            db,
            blobs,
            _lock: lock,
        };
        storage.recover_bodies()?;
        Ok(storage)
    }

    pub fn account_ids(&self) -> Result<Vec<String>, AppError> {
        let mut statement = self
            .db
            .prepare("SELECT id FROM accounts ORDER BY created_at, id")?;
        Ok(statement
            .query_map([], |row| row.get(0))?
            .collect::<Result<Vec<_>, _>>()?)
    }

    pub fn load(&self, id: &str) -> Result<Snapshot, AppError> {
        let basic = self.db.query_row("SELECT revision,display_name,email_address,provider_id,mailbox_source_id,created_at,updated_at FROM accounts WHERE id=?1", [id], |r| {
            Ok((r.get::<_, u32>(0)?,r.get::<_, String>(1)?,r.get::<_, String>(2)?,r.get::<_, Option<String>>(3)?,r.get::<_, String>(4)?,r.get::<_, String>(5)?,r.get::<_, String>(6)?))
        }).optional()?.ok_or_else(|| AppError::new(ErrorCode::NotFound))?;
        let mut profile_statement = self.db.prepare("SELECT id,generation,secret_ref,trusted_endpoints_json FROM auth_profiles WHERE account_id=?1 AND kind='password'")?;
        let records = profile_statement
            .query_map([id], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, u32>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                ))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        let profiles: Vec<AuthRecord> = records
            .into_iter()
            .map(|(id, generation, secret_ref, trust)| {
                Ok(AuthRecord {
                    id,
                    generation,
                    secret_ref,
                    trusted_endpoints: decode(&trust)?,
                })
            })
            .collect::<Result<_, AppError>>()?;
        let connection = |protocol: &str| -> Result<(ConnectionConfig, String), AppError> {
            let (host,port,tls,username,auth) = self.db.query_row("SELECT host,port,tls,username,auth_profile_id FROM mail_connections WHERE account_id=?1 AND protocol=?2", params![id,protocol], |r| Ok((r.get(0)?,r.get(1)?,r.get::<_, String>(2)?,r.get(3)?,r.get(4)?)))?;
            Ok((
                ConnectionConfig {
                    host,
                    port,
                    tls: decode(&format!("\"{tls}\""))?,
                    username,
                },
                auth,
            ))
        };
        let (imap, imap_id) = connection("imap")?;
        let (smtp, smtp_id) = connection("smtp")?;
        let find = |profile_id: &str,
                    protocol: &str,
                    connection: &ConnectionConfig|
         -> Result<AuthenticationSummary, AppError> {
            Ok(profiles
                .iter()
                .find(|p| p.id == profile_id)
                .ok_or_else(|| AppError::new(ErrorCode::StorageIncompatible))?
                .summary(&TrustEndpoint::mail(protocol, connection)))
        };
        let authentication = PasswordAuthenticationSummary {
            mode: if imap_id == smtp_id {
                AuthenticationMode::Shared
            } else {
                AuthenticationMode::Separate
            },
            imap: find(&imap_id, "imap", &imap)?,
            smtp: find(&smtp_id, "smtp", &smtp)?,
        };
        let (route_revision, route, proxy_secret) = self.load_route(id)?;
        Ok(Snapshot {
            account: Account {
                id: id.into(),
                revision: basic.0,
                display_name: basic.1,
                email_address: basic.2,
                provider_id: basic.3,
                mailbox_source_id: basic.4,
                created_at: basic.5,
                updated_at: basic.6,
                imap,
                smtp,
                authentication,
                default_route_revision: if matches!(route, RouteConfig::Inherit) {
                    Some(1)
                } else {
                    None
                },
                route,
                route_revision,
                connection_status: ConnectionStatus::Unverified,
            },
            profiles,
            proxy_secret,
        })
    }

    fn load_route(&self, id: &str) -> Result<(u32, RouteConfig, Option<ProxySecret>), AppError> {
        self.db.query_row("SELECT revision,kind,proxy_kind,host,port,username,secret_ref,generation,trusted_endpoint_json FROM network_routes WHERE account_id=?1", [id], |r| {
            let revision = r.get(0)?;
            let kind: String = r.get(1)?;
            if kind == "direct" { return Ok(Ok((revision,RouteConfig::Direct,None))); }
            if kind == "inherit" { return Ok(Ok((revision,RouteConfig::Inherit,None))); }
            let parse = || -> Result<_, AppError> {
                let proxy_kind: ProxyKind = decode(&format!("\"{}\"",r.get::<_,String>(2)?))?;
                let host: String = r.get(3)?;
                let port = r.get(4)?;
                let username: Option<String> = r.get(5)?;
                let secret_ref: Option<String> = r.get(6)?;
                let secret = if let Some(secret_ref) = secret_ref { Some(ProxySecret {
                    secret_ref,generation:r.get(7)?,trusted_endpoint:decode(&r.get::<_,String>(8)?)?
                }) } else { None };
                let authentication = username.map(|username| {
                    let endpoint = proxy_endpoint(proxy_kind,&host,port,&username);
                    ProxyAuthenticationSummary { username,password_configured:secret.is_some(),trust: if secret.as_ref().is_some_and(|s|s.trusted_endpoint == endpoint) {CredentialTrust::Trusted} else {CredentialTrust::NeedsConfirmation} }
                });
                Ok((revision,RouteConfig::Custom {proxy_kind,host,port,dns:DnsPolicy::Proxy,authentication},secret))
            };
            Ok(parse())
        })?
    }

    pub fn create_request(&self, request_id: &str) -> Result<Option<(String, String)>, AppError> {
        Ok(self
            .db
            .query_row(
                "SELECT account_id,canonical_config FROM create_requests WHERE request_id=?1",
                [request_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?)
    }

    pub fn prepare(&self, journal: &Journal) -> Result<(), AppError> {
        self.db.execute("INSERT INTO credential_journal(id,account_id,expected_revision,snapshot_json,new_refs_json,retired_refs_json,create_request_json,applied) VALUES (?1,?2,?3,?4,?5,?6,?7,0)", params![journal.id,journal.snapshot.account.id,journal.expected_revision,encode(&journal.snapshot)?,encode(&journal.new_refs)?,encode(&journal.retired_refs)?,journal.create_request.as_ref().map(encode).transpose()?])?;
        Ok(())
    }

    pub fn journals(&self) -> Result<Vec<Journal>, AppError> {
        let mut statement = self.db.prepare("SELECT id,expected_revision,snapshot_json,new_refs_json,retired_refs_json,create_request_json,applied FROM credential_journal ORDER BY rowid")?;
        let rows = statement
            .query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, Option<u32>>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, String>(4)?,
                    r.get::<_, Option<String>>(5)?,
                    r.get::<_, bool>(6)?,
                ))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        rows.into_iter()
            .map(
                |(id, expected_revision, snapshot, new_refs, retired_refs, request, applied)| {
                    Ok(Journal {
                        id,
                        expected_revision,
                        snapshot: decode(&snapshot)?,
                        new_refs: decode(&new_refs)?,
                        retired_refs: decode(&retired_refs)?,
                        create_request: request.map(|r| decode(&r)).transpose()?,
                        applied,
                    })
                },
            )
            .collect()
    }

    pub fn remove_journal(&self, id: &str) -> Result<(), AppError> {
        self.db
            .execute("DELETE FROM credential_journal WHERE id=?1", [id])?;
        Ok(())
    }

    pub fn is_referenced(&self, key: &str) -> Result<bool, AppError> {
        Ok(self.db.query_row("SELECT EXISTS(SELECT 1 FROM auth_profiles WHERE secret_ref=?1 UNION ALL SELECT 1 FROM network_routes WHERE secret_ref=?1)",[key],|r| r.get(0))?)
    }

    pub fn apply(&mut self, journal: &Journal) -> Result<(), AppError> {
        let snapshot = &journal.snapshot;
        let account = &snapshot.account;
        let tx = self.db.transaction()?;
        let current: Option<u32> = tx
            .query_row(
                "SELECT revision FROM accounts WHERE id=?1",
                [&account.id],
                |r| r.get(0),
            )
            .optional()?;
        if current != journal.expected_revision {
            return Err(AppError::new(ErrorCode::Conflict));
        }
        if current.is_none() {
            tx.execute("INSERT INTO accounts(id,revision,display_name,email_address,provider_id,mailbox_source_id,created_at,updated_at) VALUES (?1,?2,?3,?4,?5,?6,?7,?8)",params![account.id,account.revision,account.display_name,account.email_address,account.provider_id,account.mailbox_source_id,account.created_at,account.updated_at])?;
        } else {
            tx.execute("UPDATE accounts SET revision=?2,display_name=?3,email_address=?4,provider_id=?5,mailbox_source_id=?6,updated_at=?7 WHERE id=?1",params![account.id,account.revision,account.display_name,account.email_address,account.provider_id,account.mailbox_source_id,account.updated_at])?;
        }
        tx.execute(
            "DELETE FROM mail_connections WHERE account_id=?1",
            [&account.id],
        )?;
        tx.execute(
            "DELETE FROM auth_profiles WHERE account_id=?1",
            [&account.id],
        )?;
        for profile in &snapshot.profiles {
            tx.execute("INSERT INTO auth_profiles(id,account_id,kind,config_version,generation,secret_ref,trusted_endpoints_json) VALUES (?1,?2,'password',1,?3,?4,?5)",params![profile.id,account.id,profile.generation,profile.secret_ref,encode(&profile.trusted_endpoints)?])?;
        }
        for (protocol, config, auth_id) in [
            ("imap", &account.imap, &account.authentication.imap.id),
            ("smtp", &account.smtp, &account.authentication.smtp.id),
        ] {
            tx.execute("INSERT INTO mail_connections(account_id,protocol,host,port,tls,username,auth_profile_id) VALUES (?1,?2,?3,?4,?5,?6,?7)",params![account.id,protocol,config.host,config.port,enum_name(&config.tls)?,config.username,auth_id])?;
        }
        tx.execute(
            "DELETE FROM network_routes WHERE account_id=?1",
            [&account.id],
        )?;
        match &account.route {
            RouteConfig::Direct | RouteConfig::Inherit => {
                tx.execute(
                    "INSERT INTO network_routes(account_id,revision,kind) VALUES (?1,?2,?3)",
                    params![
                        account.id,
                        account.route_revision,
                        if matches!(account.route, RouteConfig::Direct) {
                            "direct"
                        } else {
                            "inherit"
                        }
                    ],
                )?;
            }
            RouteConfig::Custom {
                proxy_kind,
                host,
                port,
                dns: _,
                authentication,
            } => {
                tx.execute("INSERT INTO network_routes(account_id,revision,kind,proxy_kind,host,port,dns,username,secret_ref,generation,trusted_endpoint_json) VALUES (?1,?2,'custom',?3,?4,?5,'proxy',?6,?7,?8,?9)",params![account.id,account.route_revision,enum_name(proxy_kind)?,host,port,authentication.as_ref().map(|a|&a.username),snapshot.proxy_secret.as_ref().map(|s|&s.secret_ref),snapshot.proxy_secret.as_ref().map(|s|s.generation),snapshot.proxy_secret.as_ref().map(|s|encode(&s.trusted_endpoint)).transpose()?])?;
            }
        }
        if let Some((request_id, canonical)) = &journal.create_request {
            tx.execute("INSERT INTO create_requests(request_id,account_id,canonical_config) VALUES (?1,?2,?3)",params![request_id,account.id,canonical])?;
        }
        tx.execute(
            "UPDATE credential_journal SET applied=1 WHERE id=?1",
            [&journal.id],
        )?;
        tx.commit()?;
        Ok(())
    }
}

pub(crate) fn proxy_endpoint(
    kind: ProxyKind,
    host: &str,
    port: u16,
    username: &str,
) -> TrustEndpoint {
    TrustEndpoint {
        protocol: match kind {
            ProxyKind::Socks5 => "socks5",
            ProxyKind::HttpConnect => "httpConnect",
        }
        .into(),
        host: host.into(),
        port,
        tls: None,
        username: username.into(),
    }
}
fn enum_name(value: &impl serde::Serialize) -> Result<String, AppError> {
    let encoded = serde_json::to_value(value).map_err(|_| AppError::new(ErrorCode::Internal))?;
    encoded
        .as_str()
        .map(str::to_owned)
        .ok_or_else(|| AppError::new(ErrorCode::Internal))
}
