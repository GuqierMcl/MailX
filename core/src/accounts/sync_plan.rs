//! What a background sync needs from an account: where to connect, along which
//! route, and with which credentials. Built inside core only; nothing here is
//! ever returned to a frontend.

use super::model::TrustEndpoint;
use super::types::{CredentialTrust, RouteConfig};
use super::{AccountService, validate_uuid};
use crate::error::{AppError, ErrorCode};
use crate::imap::ImapLogin;
use crate::mail::Folder;
use crate::net::{NetRoute, ProxyRoute};
use crate::secrets::SecretStoreError;
use crate::storage::proxy_endpoint;
use crate::sync::folders::FolderRecord;

#[derive(Debug)]
pub(crate) struct SyncPlan {
    pub mailbox_source_id: String,
    pub login: ImapLogin,
    pub route: NetRoute,
}

pub(crate) struct SyncIdentity {
    pub mailbox_source_id: String,
    pub revision: u32,
}

#[derive(Debug)]
pub(crate) enum PlanError {
    Account(AppError),
    /// The stored password was confirmed for different server or proxy
    /// settings; using it here could send it somewhere the user never approved.
    CredentialUntrusted,
    CredentialMissing,
}

impl From<AppError> for PlanError {
    fn from(error: AppError) -> Self {
        Self::Account(error)
    }
}

impl From<SecretStoreError> for PlanError {
    fn from(error: SecretStoreError) -> Self {
        Self::Account(error.into())
    }
}

impl AccountService {
    /// Guard cached work using durable metadata, without consulting the host's
    /// credential store. Credentials are resolved only when building a sync plan.
    pub(crate) fn sync_identity(&self, account_id: &str) -> Result<SyncIdentity, AppError> {
        validate_uuid(account_id, "accountId")?;
        let account = self.storage.load(account_id)?.account;
        Ok(SyncIdentity {
            mailbox_source_id: account.mailbox_source_id,
            revision: account.revision,
        })
    }

    /// Resolves credentials for an IMAP sync. A credential is only used for the
    /// exact endpoint it was confirmed for; anything else needs re-entry.
    pub(crate) fn sync_plan(&self, account_id: &str) -> Result<SyncPlan, PlanError> {
        let snapshot = self.storage.load(account_id)?;
        let account = &snapshot.account;

        let endpoint = TrustEndpoint::mail("imap", &account.imap);
        let profile = snapshot
            .profiles
            .iter()
            .find(|profile| profile.id == account.authentication.imap.id)
            .ok_or_else(|| AppError::new(ErrorCode::StorageIncompatible))?;
        if profile.trust(&endpoint) != CredentialTrust::Trusted {
            return Err(PlanError::CredentialUntrusted);
        }
        let password = self
            .secrets
            .get(&profile.secret_ref)?
            .ok_or(PlanError::CredentialMissing)?;

        let route = self.image_route(account_id)?;

        Ok(SyncPlan {
            mailbox_source_id: account.mailbox_source_id.clone(),
            login: ImapLogin {
                host: account.imap.host.clone(),
                port: account.imap.port,
                tls: account.imap.tls,
                username: account.imap.username.clone(),
                password,
            },
            route,
        })
    }

    /// Resolves only proxy credentials; image requests never read mail credentials.
    pub(crate) fn image_route(&self, account_id: &str) -> Result<NetRoute, PlanError> {
        let snapshot = self.storage.load(account_id)?;
        let account = &snapshot.account;
        let route = match &account.route {
            // The application default route is direct until a default proxy setting exists.
            RouteConfig::Direct | RouteConfig::Inherit => NetRoute::default(),
            RouteConfig::Custom {
                proxy_kind,
                host,
                port,
                authentication,
                ..
            } => {
                let auth = match authentication {
                    None => None,
                    Some(summary) => {
                        let secret = snapshot
                            .proxy_secret
                            .as_ref()
                            .ok_or(PlanError::CredentialMissing)?;
                        if secret.trusted_endpoint
                            != proxy_endpoint(*proxy_kind, host, *port, &summary.username)
                        {
                            return Err(PlanError::CredentialUntrusted);
                        }
                        let password = self
                            .secrets
                            .get(&secret.secret_ref)?
                            .ok_or(PlanError::CredentialMissing)?;
                        Some((summary.username.clone(), password))
                    }
                };
                NetRoute {
                    proxy: Some(ProxyRoute {
                        kind: *proxy_kind,
                        host: host.clone(),
                        port: *port,
                        auth,
                    }),
                }
            }
        };
        Ok(route)
    }

    /// Stores a folder listing taken with `sync_plan`. Returns `false` when the
    /// account changed servers or was removed meanwhile, so the listing is stale.
    pub(crate) fn store_folders(
        &mut self,
        account_id: &str,
        mailbox_source_id: &str,
        records: &[FolderRecord],
    ) -> Result<bool, AppError> {
        let now = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
        self.storage
            .replace_folders(account_id, mailbox_source_id, records, &now)
    }

    /// Folders last synced from the account's current server.
    pub fn list_folders(&self, account_id: &str) -> Result<Vec<Folder>, AppError> {
        super::validate_uuid(account_id, "accountId")?;
        self.storage.load(account_id)?;
        self.storage.list_folders(account_id)
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use std::sync::Arc;

    use serde_json::{Value, json};
    use tempfile::TempDir;
    use uuid::Uuid;

    use super::*;
    use crate::accounts::types::{Account, CreateAccountRequest, UpdateConnectionRequest};
    use crate::imap::FolderStatus;
    use crate::mail::FolderRole;
    use crate::secrets::test_support::MemorySecrets;

    pub(crate) fn config(route: Value) -> Value {
        json!({"displayName":"Work","emailAddress":"lin@example.com","providerId":null,
            "imap":{"host":"imap.example.com","port":993,"tls":"implicit","username":"lin"},
            "smtp":{"host":"smtp.example.com","port":465,"tls":"implicit","username":"lin"},
            "authentication":{"kind":"password","mode":"shared","password":{"action":"replace","value":"mail-secret"}},
            "route":route})
    }

    fn setup(route: Value) -> (TempDir, AccountService, Account) {
        let directory = TempDir::new().unwrap();
        let mut service =
            AccountService::open(directory.path(), Arc::new(MemorySecrets::default())).unwrap();
        let request: CreateAccountRequest = serde_json::from_value(
            json!({"requestId": Uuid::new_v4().to_string(), "config": config(route)}),
        )
        .unwrap();
        let account = service.create(request).unwrap();
        (directory, service, account)
    }

    /// A connection update built from a full account config: the profile
    /// fields belong to their own request and are dropped here.
    pub(crate) fn connection_update(
        account_id: &str,
        expected_revision: u32,
        mut value: Value,
    ) -> UpdateConnectionRequest {
        let object = value.as_object_mut().unwrap();
        object.remove("displayName");
        object.remove("color");
        serde_json::from_value(json!({
            "accountId": account_id, "expectedRevision": expected_revision, "connection": value,
        }))
        .unwrap()
    }

    fn update(
        service: &mut AccountService,
        account: &Account,
        change: impl FnOnce(&mut Value),
    ) -> Account {
        let mut value = config(json!({"kind":"direct"}));
        value["authentication"]["password"] = json!({"action":"keep"});
        change(&mut value);
        service
            .update_connection(connection_update(&account.id, account.revision, value))
            .unwrap()
    }

    fn record(
        remote_name: &str,
        parent: Option<&str>,
        role: FolderRole,
        unseen: u32,
    ) -> FolderRecord {
        FolderRecord {
            remote_name: remote_name.into(),
            parent_remote_name: parent.map(Into::into),
            name: remote_name.rsplit('/').next().unwrap().into(),
            delimiter: Some("/".into()),
            role,
            selectable: true,
            attributes: vec![],
            status: Some(FolderStatus {
                messages: 10,
                unseen,
                uid_next: Some(11),
                uid_validity: Some(1),
            }),
        }
    }

    #[test]
    fn plan_carries_the_imap_login_and_direct_route() {
        let (_directory, service, account) = setup(json!({"kind":"direct"}));
        let plan = service.sync_plan(&account.id).unwrap();
        assert_eq!(plan.mailbox_source_id, account.mailbox_source_id);
        assert_eq!(
            (
                plan.login.host.as_str(),
                plan.login.port,
                plan.login.username.as_str()
            ),
            ("imap.example.com", 993, "lin")
        );
        assert_eq!(plan.login.password.expose_secret(), "mail-secret");
        assert!(plan.route.proxy.is_none());
    }

    #[test]
    fn plan_includes_the_custom_proxy_and_its_credentials() {
        let (_directory, service, account) = setup(
            json!({"kind":"custom","proxyKind":"socks5","host":"127.0.0.1","port":1080,"dns":"proxy",
            "authentication":{"username":"proxy-user","password":{"action":"replace","value":"proxy-secret"}}}),
        );
        let proxy = service.sync_plan(&account.id).unwrap().route.proxy.unwrap();
        assert_eq!((proxy.host.as_str(), proxy.port), ("127.0.0.1", 1080));
        let (username, password) = proxy.auth.unwrap();
        assert_eq!(
            (username.as_str(), password.expose_secret()),
            ("proxy-user", "proxy-secret")
        );
    }

    #[test]
    fn a_changed_server_does_not_receive_the_old_password() {
        let (_directory, mut service, account) = setup(json!({"kind":"direct"}));
        update(&mut service, &account, |value| {
            value["imap"]["host"] = json!("imap.attacker.example")
        });
        assert!(matches!(
            service.sync_plan(&account.id),
            Err(PlanError::CredentialUntrusted)
        ));
    }

    #[test]
    fn folders_keep_their_ids_and_follow_the_server_listing() {
        let (_directory, mut service, account) = setup(json!({"kind":"direct"}));
        let source = account.mailbox_source_id.clone();
        assert!(service.list_folders(&account.id).unwrap().is_empty());

        let first = [
            record("INBOX", None, FolderRole::Inbox, 3),
            record("Projects", None, FolderRole::Other, 0),
            record("Projects/MailX", Some("Projects"), FolderRole::Other, 1),
        ];
        assert!(service.store_folders(&account.id, &source, &first).unwrap());
        let folders = service.list_folders(&account.id).unwrap();
        let by_name = |folders: &[Folder], name: &str| {
            folders
                .iter()
                .find(|folder| folder.name == name)
                .cloned()
                .unwrap()
        };
        let inbox = by_name(&folders, "INBOX");
        assert_eq!(
            (inbox.role, inbox.unread_count, inbox.total_count),
            (FolderRole::Inbox, 3, 10)
        );
        assert_eq!(
            by_name(&folders, "MailX").parent_id,
            Some(by_name(&folders, "Projects").id)
        );

        // Next listing: Projects/MailX is gone, Archive is new, INBOX changed counts.
        let second = [
            record("INBOX", None, FolderRole::Inbox, 5),
            record("Archive", None, FolderRole::Archive, 0),
        ];
        assert!(
            service
                .store_folders(&account.id, &source, &second)
                .unwrap()
        );
        let folders = service.list_folders(&account.id).unwrap();
        let mut names: Vec<_> = folders.iter().map(|folder| folder.name.as_str()).collect();
        names.sort();
        assert_eq!(names, ["Archive", "INBOX"]);
        assert_eq!(
            by_name(&folders, "INBOX").id,
            inbox.id,
            "local ids survive resyncs"
        );
        assert_eq!(by_name(&folders, "INBOX").unread_count, 5);
    }

    #[test]
    fn listings_from_a_previous_server_are_discarded() {
        let (_directory, mut service, account) = setup(json!({"kind":"direct"}));
        let old_source = account.mailbox_source_id.clone();
        assert!(
            service
                .store_folders(
                    &account.id,
                    &old_source,
                    &[record("INBOX", None, FolderRole::Inbox, 1)]
                )
                .unwrap()
        );

        let moved = update(&mut service, &account, |value| {
            value["imap"]["host"] = json!("imap.other.example");
            value["authentication"]["password"] = json!({"action":"replace","value":"new-secret"});
        });
        assert_ne!(moved.mailbox_source_id, old_source);
        assert!(
            service.list_folders(&account.id).unwrap().is_empty(),
            "old server folders are hidden"
        );
        assert!(
            !service
                .store_folders(
                    &account.id,
                    &old_source,
                    &[record("Stale", None, FolderRole::Other, 0)]
                )
                .unwrap(),
            "a late result from the old server is ignored"
        );
        assert!(service.list_folders(&account.id).unwrap().is_empty());
    }
}
