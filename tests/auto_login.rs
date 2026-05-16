use std::{
    fs,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

use wiremock::{
    Mock, MockServer, Request, Respond, ResponseTemplate,
    matchers::{body_string, method, path},
};
use xjtuportal::{
    RunStatus,
    config::{
        AccountConfig, AppConfig, InterfaceConfig, KnownMacConfig, LogoutConfig, NetworkConfig,
        TargetConfig,
    },
    crypto::encrypt_text,
    list_default_sessions, logout_default_session, run,
};

#[tokio::test]
async fn automatic_logout_then_retry_succeeds() {
    let server = MockServer::start().await;
    let login_calls = Arc::new(AtomicUsize::new(0));

    Mock::given(method("GET"))
        .and(path("/probe"))
        .respond_with(
            ResponseTemplate::new(302).insert_header("Location", "http://10.184.6.32/portal"),
        )
        .expect(1)
        .mount(&server)
        .await;

    Mock::given(method("POST"))
        .and(path("/portal-conversion/api/v3/portal/connect"))
        .respond_with(SequentialLogin {
            calls: login_calls.clone(),
        })
        .expect(2)
        .mount(&server)
        .await;

    Mock::given(method("POST"))
        .and(path("/portal-conversion/api/v3/session/list"))
        .respond_with(
            ResponseTemplate::new(200).set_body_string(
                encrypt_text(
                    &serde_json::json!({
                        "concurrency": "3",
                        "sessions": [
                            {
                                "framed_ip_address": "10.180.0.2",
                                "calling_station_id": "00-00-5e-00-53-01",
                                "acct_start_time": "2026-05-15 12:00:00",
                                "acct_unique_id": "logout-me"
                            },
                            {
                                "framed_ip_address": "10.180.0.3",
                                "calling_station_id": "11:22:33:44:55:66",
                                "acct_start_time": "2026-05-15 12:01:00",
                                "acct_unique_id": "keep-me"
                            }
                        ]
                    })
                    .to_string(),
                )
                .unwrap(),
            ),
        )
        .expect(1)
        .mount(&server)
        .await;

    Mock::given(method("POST"))
        .and(path("/portal-conversion/api/v3/session/acctUniqueId"))
        .and(body_string(
            encrypt_text(
                &serde_json::json!({
                    "acctUniqueId": "logout-me",
                    "mac": "00-00-5e-00-53-01"
                })
                .to_string(),
            )
            .unwrap(),
        ))
        .respond_with(ResponseTemplate::new(200))
        .expect(1)
        .mount(&server)
        .await;

    let config = AppConfig {
        network: NetworkConfig {
            gateway: server.uri(),
            nas_ip: Some("127.0.0.1".to_string()),
            test_url: format!("{}/probe", server.uri()),
            login_url: format!("{}/portal-conversion/api/v3/portal/connect", server.uri()),
            session_login_redirect_url: format!("{}/wenet/auth", server.uri()),
            session_list_url: format!("{}/portal-conversion/api/v3/session/list", server.uri()),
            session_logout_url: format!(
                "{}/portal-conversion/api/v3/session/acctUniqueId",
                server.uri()
            ),
            timeout_secs: 5,
        },
        default_account: Some(AccountConfig {
            id: None,
            username: "u@xjtu".to_string(),
            password: "p".to_string(),
        }),
        logout: LogoutConfig {
            enabled: true,
            current_mac: None,
            known_macs: vec![KnownMacConfig::new("11:22:33:44:55:66", None)],
        },
        accounts: vec![],
        interfaces: vec![],
        targets: vec![],
    };

    let status = run(config, None).await.unwrap();

    assert_eq!(status, RunStatus::Success);
    assert_eq!(login_calls.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn already_online_does_not_login() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/probe"))
        .respond_with(ResponseTemplate::new(200))
        .expect(1)
        .mount(&server)
        .await;

    let config = AppConfig {
        network: NetworkConfig {
            gateway: server.uri(),
            nas_ip: Some("127.0.0.1".to_string()),
            test_url: format!("{}/probe", server.uri()),
            login_url: format!("{}/portal-conversion/api/v3/portal/connect", server.uri()),
            session_login_redirect_url: format!("{}/wenet/auth", server.uri()),
            session_list_url: format!("{}/portal-conversion/api/v3/session/list", server.uri()),
            session_logout_url: format!(
                "{}/portal-conversion/api/v3/session/acctUniqueId",
                server.uri()
            ),
            timeout_secs: 5,
        },
        default_account: Some(AccountConfig {
            id: None,
            username: "u@xjtu".to_string(),
            password: "p".to_string(),
        }),
        logout: LogoutConfig::default(),
        accounts: vec![],
        interfaces: vec![],
        targets: vec![],
    };

    let status = run(config, None).await.unwrap();

    assert_eq!(status, RunStatus::Success);
}

#[tokio::test]
async fn run_updates_nas_ip_from_captive_redirect() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/probe"))
        .respond_with(ResponseTemplate::new(302).insert_header(
            "Location",
            "http://10.184.6.32:80?userip=10.180.18.119&nasip=10.6.33.10&usermac=7e:6d:f0:76:ec:a4",
        ))
        .expect(1)
        .mount(&server)
        .await;
    mount_success_login(&server).await;

    let config_file = tempfile::NamedTempFile::new().unwrap();
    fs::write(
        config_file.path(),
        r#"
        [network]
        gateway = "10.184.6.32"

        [default_account]
        username = "u@xjtu"
        password = "p"
        "#,
    )
    .unwrap();

    let mut config = default_config(&server, LogoutConfig::default());
    config.network.nas_ip = None;
    let status = run(config, Some(config_file.path().to_path_buf()))
        .await
        .unwrap();

    let updated = AppConfig::read(config_file.path()).unwrap();
    assert_eq!(status, RunStatus::Success);
    assert_eq!(updated.network.nas_ip.as_deref(), Some("10.6.33.10"));
}

#[tokio::test]
async fn list_default_sessions_applies_known_names() {
    let server = MockServer::start().await;
    mount_redirect_probe(&server).await;
    mount_success_login(&server).await;
    mount_session_list(&server).await;

    let sessions = list_default_sessions(
        default_config(
            &server,
            LogoutConfig {
                enabled: false,
                current_mac: None,
                known_macs: vec![KnownMacConfig::new(
                    "11:22:33:44:55:66",
                    Some("phone".to_string()),
                )],
            },
        ),
        None,
    )
    .await
    .unwrap();

    assert_eq!(sessions.len(), 2);
    assert_eq!(sessions[0].name, "unknown");
    assert_eq!(sessions[1].name, "phone");
}

#[tokio::test]
async fn list_default_sessions_uses_session_redirect_url_when_online() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/probe"))
        .respond_with(ResponseTemplate::new(200))
        .expect(1)
        .mount(&server)
        .await;

    Mock::given(method("POST"))
        .and(path("/portal-conversion/api/v3/portal/connect"))
        .respond_with(AssertLoginBody {
            expected_redirect_url: format!(
                "{}/wenet/auth?userip=127.0.0.1&nasip=127.0.0.1&",
                server.uri()
            ),
        })
        .expect(1)
        .mount(&server)
        .await;

    mount_session_list(&server).await;

    let sessions = list_default_sessions(default_config(&server, LogoutConfig::default()), None)
        .await
        .unwrap();

    assert_eq!(sessions.len(), 2);
}

#[tokio::test]
async fn logout_default_session_accepts_known_name() {
    let server = MockServer::start().await;
    mount_redirect_probe(&server).await;
    mount_success_login(&server).await;
    mount_session_list(&server).await;

    Mock::given(method("POST"))
        .and(path("/portal-conversion/api/v3/session/acctUniqueId"))
        .and(body_string(
            encrypt_text(
                &serde_json::json!({
                    "acctUniqueId": "keep-me",
                    "mac": "11:22:33:44:55:66"
                })
                .to_string(),
            )
            .unwrap(),
        ))
        .respond_with(ResponseTemplate::new(200))
        .expect(1)
        .mount(&server)
        .await;

    let session = logout_default_session(
        default_config(
            &server,
            LogoutConfig {
                enabled: false,
                current_mac: None,
                known_macs: vec![KnownMacConfig::new(
                    "11:22:33:44:55:66",
                    Some("phone".to_string()),
                )],
            },
        ),
        Some("phone"),
        None,
    )
    .await
    .unwrap();

    assert_eq!(session.name, "phone");
    assert_eq!(session.mac, "11:22:33:44:55:66");
}

#[tokio::test]
async fn logout_default_session_uses_configured_current_mac() {
    let server = MockServer::start().await;
    mount_redirect_probe(&server).await;
    mount_success_login(&server).await;
    mount_session_list(&server).await;

    Mock::given(method("POST"))
        .and(path("/portal-conversion/api/v3/session/acctUniqueId"))
        .and(body_string(
            encrypt_text(
                &serde_json::json!({
                    "acctUniqueId": "logout-me",
                    "mac": "00-00-5e-00-53-01"
                })
                .to_string(),
            )
            .unwrap(),
        ))
        .respond_with(ResponseTemplate::new(200))
        .expect(1)
        .mount(&server)
        .await;

    let session = logout_default_session(
        default_config(
            &server,
            LogoutConfig {
                enabled: false,
                current_mac: Some("00:00:5e:00:53:01".to_string()),
                known_macs: vec![],
            },
        ),
        None,
        None,
    )
    .await
    .unwrap();

    assert_eq!(session.mac, "00:00:5e:00:53:01");
}

#[derive(Clone)]
struct SequentialLogin {
    calls: Arc<AtomicUsize>,
}

struct AssertLoginBody {
    expected_redirect_url: String,
}

async fn mount_redirect_probe(server: &MockServer) {
    Mock::given(method("GET"))
        .and(path("/probe"))
        .respond_with(
            ResponseTemplate::new(302).insert_header("Location", "http://10.184.6.32/portal"),
        )
        .expect(1)
        .mount(server)
        .await;
}

async fn mount_success_login(server: &MockServer) {
    Mock::given(method("POST"))
        .and(path("/portal-conversion/api/v3/portal/connect"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string(encrypt_text(r#"{"code":0,"token":"session-token"}"#).unwrap()),
        )
        .expect(1)
        .mount(server)
        .await;
}

async fn mount_session_list(server: &MockServer) {
    Mock::given(method("POST"))
        .and(path("/portal-conversion/api/v3/session/list"))
        .respond_with(
            ResponseTemplate::new(200).set_body_string(
                encrypt_text(
                    &serde_json::json!({
                        "concurrency": "3",
                        "sessions": [
                            {
                                "deviceType": "PC",
                                "framed_ip_address": "10.180.0.2",
                                "calling_station_id": "00-00-5e-00-53-01",
                                "acct_start_time": "2026-05-15 12:00:00",
                                "acct_unique_id": "logout-me"
                            },
                            {
                                "deviceType": "phone",
                                "framed_ip_address": "10.180.0.3",
                                "calling_station_id": "11:22:33:44:55:66",
                                "acct_start_time": "2026-05-15 12:01:00",
                                "acct_unique_id": "keep-me"
                            }
                        ]
                    })
                    .to_string(),
                )
                .unwrap(),
            ),
        )
        .expect(1)
        .mount(server)
        .await;
}

fn default_config(server: &MockServer, logout: LogoutConfig) -> AppConfig {
    AppConfig {
        network: NetworkConfig {
            gateway: server.uri(),
            nas_ip: Some("127.0.0.1".to_string()),
            test_url: format!("{}/probe", server.uri()),
            login_url: format!("{}/portal-conversion/api/v3/portal/connect", server.uri()),
            session_login_redirect_url: format!("{}/wenet/auth", server.uri()),
            session_list_url: format!("{}/portal-conversion/api/v3/session/list", server.uri()),
            session_logout_url: format!(
                "{}/portal-conversion/api/v3/session/acctUniqueId",
                server.uri()
            ),
            timeout_secs: 5,
        },
        default_account: Some(AccountConfig {
            id: None,
            username: "u@xjtu".to_string(),
            password: "p".to_string(),
        }),
        logout,
        accounts: vec![],
        interfaces: vec![],
        targets: vec![],
    }
}

impl Respond for SequentialLogin {
    fn respond(&self, _request: &Request) -> ResponseTemplate {
        let call = self.calls.fetch_add(1, Ordering::SeqCst);
        let json = if call == 0 {
            r#"{"error":81,"errorDescription":"already have 3 sessions","token":"session-token"}"#
        } else {
            r#"{"code":0,"token":"abcdef123456"}"#
        };
        ResponseTemplate::new(200).set_body_string(encrypt_text(json).unwrap())
    }
}

impl Respond for AssertLoginBody {
    fn respond(&self, request: &Request) -> ResponseTemplate {
        let body = std::str::from_utf8(&request.body)
            .ok()
            .and_then(|body| xjtuportal::crypto::decrypt_json::<serde_json::Value>(body).ok())
            .unwrap();

        assert_eq!(body["redirectUrl"], self.expected_redirect_url);
        ResponseTemplate::new(200)
            .set_body_string(encrypt_text(r#"{"code":0,"token":"session-token"}"#).unwrap())
    }
}

#[tokio::test]
async fn multi_target_continues_after_one_target_fails() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/probe"))
        .respond_with(ResponseTemplate::new(200))
        .expect(1)
        .mount(&server)
        .await;

    let config = AppConfig {
        network: NetworkConfig {
            gateway: server.uri(),
            nas_ip: Some("127.0.0.1".to_string()),
            test_url: format!("{}/probe", server.uri()),
            login_url: format!("{}/portal-conversion/api/v3/portal/connect", server.uri()),
            session_login_redirect_url: format!("{}/wenet/auth", server.uri()),
            session_list_url: format!("{}/portal-conversion/api/v3/session/list", server.uri()),
            session_logout_url: format!(
                "{}/portal-conversion/api/v3/session/acctUniqueId",
                server.uri()
            ),
            timeout_secs: 5,
        },
        default_account: None,
        logout: LogoutConfig::default(),
        accounts: vec![AccountConfig {
            id: Some("main".to_string()),
            username: "u@xjtu".to_string(),
            password: "p".to_string(),
        }],
        interfaces: vec![InterfaceConfig {
            id: "bad-iface".to_string(),
            name: Some("definitely-not-a-real-interface".to_string()),
            local_ip: None,
            mac: None,
        }],
        targets: vec![
            TargetConfig {
                id: "bad".to_string(),
                account: "main".to_string(),
                interface: Some("bad-iface".to_string()),
            },
            TargetConfig {
                id: "good".to_string(),
                account: "main".to_string(),
                interface: None,
            },
        ],
    };

    let status = run(config, None).await.unwrap();

    assert_eq!(status, RunStatus::PartialFailure);
}
