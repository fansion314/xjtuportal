use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};

use wiremock::{
    matchers::{method, path},
    Mock, MockServer, Request, Respond, ResponseTemplate,
};
use xjtuportal::{
    config::{
        AccountConfig, AppConfig, InterfaceConfig, LogoutConfig, NetworkConfig, TargetConfig,
    },
    crypto::encrypt_text,
    run, RunStatus,
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
        .and(path("/portal/api/v2/online"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "token": "session-token"
        })))
        .expect(2)
        .mount(&server)
        .await;

    Mock::given(method("GET"))
        .and(path("/portal/api/v2/session/list"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "concurrency": "3",
            "sessions": [
                {
                    "framed_ip_address": "10.180.0.2",
                    "calling_station_id": "00:00:5e:00:53:01",
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
        })))
        .expect(1)
        .mount(&server)
        .await;

    Mock::given(method("DELETE"))
        .and(path("/portal/api/v2/session/acctUniqueId/logout-me"))
        .respond_with(ResponseTemplate::new(200))
        .expect(1)
        .mount(&server)
        .await;

    let config = AppConfig {
        network: NetworkConfig {
            gateway: server.uri(),
            test_url: format!("{}/probe", server.uri()),
            login_url: format!("{}/portal-conversion/api/v3/portal/connect", server.uri()),
            timeout_secs: 5,
        },
        default_account: Some(AccountConfig {
            id: None,
            username: "u@xjtu".to_string(),
            password: "p".to_string(),
        }),
        logout: LogoutConfig {
            enabled: true,
            known_macs: vec!["11:22:33:44:55:66".to_string()],
        },
        accounts: vec![],
        interfaces: vec![],
        targets: vec![],
    };

    let status = run(config).await.unwrap();

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
            test_url: format!("{}/probe", server.uri()),
            login_url: format!("{}/portal-conversion/api/v3/portal/connect", server.uri()),
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

    let status = run(config).await.unwrap();

    assert_eq!(status, RunStatus::Success);
}

#[derive(Clone)]
struct SequentialLogin {
    calls: Arc<AtomicUsize>,
}

impl Respond for SequentialLogin {
    fn respond(&self, _request: &Request) -> ResponseTemplate {
        let call = self.calls.fetch_add(1, Ordering::SeqCst);
        let json = if call == 0 {
            r#"{"error":81,"errorDescription":"already have 3 sessions"}"#
        } else {
            r#"{"code":0,"token":"abcdef123456"}"#
        };
        ResponseTemplate::new(200).set_body_string(encrypt_text(json).unwrap())
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
            test_url: format!("{}/probe", server.uri()),
            login_url: format!("{}/portal-conversion/api/v3/portal/connect", server.uri()),
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

    let status = run(config).await.unwrap();

    assert_eq!(status, RunStatus::PartialFailure);
}
