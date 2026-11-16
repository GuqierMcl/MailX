//! Pure preparation for testing credentials before an account exists. This
//! module has no account service, storage, or host credential-store access.
use super::types::{
    AuthenticationInput, ConnectionInput, PasswordAuthenticationInput, RouteInput, SecretUpdate,
};
use super::{validate_connection, validate_create_secrets};
use crate::error::{AppError, ErrorCode};
use crate::imap::ImapLogin;
use crate::net::{NetRoute, ProxyRoute};
use crate::secrets::SecretValue;
use crate::smtp::SmtpLogin;

pub(crate) enum DraftConnectionPlan {
    ImapSmtp {
        route: NetRoute,
        imap: ImapLogin,
        smtp: SmtpLogin,
    },
    Ews {
        route: NetRoute,
        endpoint: crate::ews::Endpoint,
        user: String,
        password: SecretValue,
    },
}

pub(crate) fn route_from_input(mut input: RouteInput) -> Result<NetRoute, AppError> {
    super::validate_route(&mut input)?;
    Ok(match input {
        RouteInput::Direct | RouteInput::Inherit => NetRoute::default(),
        RouteInput::Custom {
            proxy_kind,
            host,
            port,
            authentication,
            ..
        } => NetRoute {
            image_proxy_dns: false,
            proxy: Some(ProxyRoute {
                kind: proxy_kind,
                host,
                port,
                auth: authentication
                    .map(|auth| Ok::<_, AppError>((auth.username, replacement(auth.password)?)))
                    .transpose()?,
            }),
        },
    })
}

pub(crate) fn prepare(mut input: ConnectionInput) -> Result<DraftConnectionPlan, AppError> {
    validate_connection(&mut input)?;
    validate_create_secrets(&input)?;
    let AuthenticationInput::Password { credentials } = input.authentication else {
        return Err(AppError::new(ErrorCode::Unsupported));
    };
    let (receive_password, submit_password) = match credentials {
        PasswordAuthenticationInput::Shared { password } => {
            let password = replacement(password)?;
            (password.clone(), password)
        }
        PasswordAuthenticationInput::Separate {
            imap_password,
            smtp_password,
        } => (replacement(imap_password)?, replacement(smtp_password)?),
    };
    let route = route_from_input(input.route)?;
    Ok(match input.connection {
        super::MailConnection::ImapSmtp { imap, smtp } => DraftConnectionPlan::ImapSmtp {
            route,
            imap: ImapLogin {
                host: imap.host,
                port: imap.port,
                tls: imap.tls,
                username: imap.username,
                credential: crate::imap::ProtocolCredential::Password(receive_password),
            },
            smtp: SmtpLogin {
                host: smtp.host,
                port: smtp.port,
                tls: smtp.tls,
                username: smtp.username,
                credential: crate::imap::ProtocolCredential::Password(submit_password),
            },
        },
        super::MailConnection::Ews { ews, .. } => DraftConnectionPlan::Ews {
            route,
            endpoint: crate::ews::Endpoint::parse(&ews.url)
                .map_err(|_| AppError::validation("ews.url"))?,
            user: ews.username,
            password: receive_password,
        },
    })
}

fn replacement(update: SecretUpdate) -> Result<SecretValue, AppError> {
    match update {
        SecretUpdate::Replace { value } => Ok(value),
        SecretUpdate::Keep => Err(AppError::validation("authentication")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::accounts::sync_plan::tests::config;
    use serde_json::{Value, json};

    fn connection(mut config: Value) -> ConnectionInput {
        config.as_object_mut().unwrap().remove("displayName");
        config.as_object_mut().unwrap().remove("color");
        serde_json::from_value(config).unwrap()
    }

    #[test]
    fn draft_preparation_uses_create_validation_and_normalizes_each_endpoint() {
        let mut input = config(json!({"kind":"inherit"}));
        input["emailAddress"] = json!("  Fixture@example.test  ");
        input["imap"]["host"] = json!(" IMAP.EXAMPLE.TEST ");
        input["imap"]["username"] = json!("  \u{feff}  ");
        input["smtp"]["username"] = json!(" SMTP-user ");
        input["authentication"] = json!({
            "kind":"password", "mode":"separate",
            "imapPassword":{"action":"replace","value":" imap-secret "},
            "smtpPassword":{"action":"replace","value":" smtp-secret "}
        });
        let DraftConnectionPlan::ImapSmtp { route, imap, smtp } =
            prepare(connection(input)).unwrap()
        else {
            panic!("IMAP fixture")
        };
        assert_eq!(imap.host, "imap.example.test");
        assert_eq!(imap.username, "Fixture@example.test");
        assert_eq!(smtp.username, "SMTP-user");
        assert_eq!(
            imap.credential.secret().unwrap().expose_secret(),
            " imap-secret "
        );
        assert_eq!(
            smtp.credential.secret().unwrap().expose_secret(),
            " smtp-secret "
        );
        assert!(route.proxy.is_none());
    }

    #[test]
    fn keep_is_rejected_for_shared_separate_and_proxy_credentials() {
        let shared = json!({"kind":"password","mode":"shared","password":{"action":"keep"}});
        for authentication in [
            shared,
            json!({"kind":"password","mode":"separate", "imapPassword":{"action":"keep"}, "smtpPassword":{"action":"replace","value":"secret"}}),
            json!({"kind":"password","mode":"separate", "imapPassword":{"action":"replace","value":"secret"}, "smtpPassword":{"action":"keep"}}),
        ] {
            let mut input = config(json!({"kind":"direct"}));
            input["authentication"] = authentication;
            assert_eq!(
                prepare(connection(input)).err().unwrap(),
                AppError::validation("authentication")
            );
        }
        let input = config(json!({
            "kind":"custom", "proxyKind":"socks5", "host":"127.0.0.1", "port":1080, "dns":"proxy",
            "authentication":{"username":"proxy-user","password":{"action":"keep"}}
        }));
        assert_eq!(
            prepare(connection(input)).err().unwrap(),
            AppError::validation("route.authentication")
        );
    }

    #[test]
    fn custom_proxy_credentials_remain_in_the_mail_route_only() {
        let input = config(json!({
            "kind":"custom", "proxyKind":"httpConnect", "host":" PROXY.EXAMPLE.TEST ", "port":8080, "dns":"proxy",
            "imageProxyDns":true, "confirmImageProxyDns":true,
            "authentication":{"username":" proxy-user ","password":{"action":"replace","value":" proxy-secret "}}
        }));
        let DraftConnectionPlan::ImapSmtp { route, .. } = prepare(connection(input)).unwrap()
        else {
            panic!("IMAP fixture")
        };
        assert!(!route.image_proxy_dns);
        let proxy = route.proxy.unwrap();
        assert_eq!(proxy.host, "proxy.example.test");
        assert_eq!(proxy.port, 8080);
        let (username, password) = proxy.auth.unwrap();
        assert_eq!(username, " proxy-user ");
        assert_eq!(password.expose_secret(), " proxy-secret ");
    }
}
