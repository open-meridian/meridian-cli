use super::*;

/// RFC 7636, appendix B.
const VERIFIER: &str = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
const CHALLENGE: &str = "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM";

#[test]
fn the_challenge_is_the_rfc_7636_one() {
    assert_eq!(challenge_for(VERIFIER), CHALLENGE);
    let fresh = Pkce::new();
    assert_eq!(fresh.verifier.len(), 43, "32 random bytes");
    assert_eq!(fresh.challenge, challenge_for(&fresh.verifier));
    assert_ne!(Pkce::new().verifier, fresh.verifier);
}

#[test]
fn a_session_goes_over_https_or_stays_on_this_machine() {
    for good in [
        "https://dash.firm.example",
        "https://dash.firm.example:8443/",
        "http://127.0.0.1:8443",
        "http://localhost:8443",
        "http://[::1]:8443",
    ] {
        assert!(address(good).is_ok(), "{good}");
    }
    assert_eq!(
        address("https://dash.firm.example/").unwrap(),
        "https://dash.firm.example"
    );
    for bad in [
        "http://dash.firm.example",
        "http://127.0.0.1.attacker.example:8443",
        "dash.firm.example",
        "ftp://dash.firm.example",
        "https://dash.firm.example/admin",
        "https://dash.firm.example?x=1",
    ] {
        assert!(address(bad).is_err(), "{bad}");
    }
}

#[test]
fn the_authorisation_carries_the_client_the_challenge_the_callback_and_the_resource() {
    let pkce = Pkce {
        verifier: VERIFIER.into(),
        challenge: CHALLENGE.into(),
    };
    assert_eq!(
        authorize_url(
            "https://dash.firm.example",
            "mdc_laptop",
            "http://127.0.0.1:53682/callback",
            &pkce,
            "st-1"
        ),
        format!(
            "https://dash.firm.example/oauth/authorize?response_type=code&client_id=mdc_laptop\
             &redirect_uri=http%3A%2F%2F127.0.0.1%3A53682%2Fcallback&code_challenge={CHALLENGE}\
             &code_challenge_method=S256&state=st-1\
             &resource=https%3A%2F%2Fdash.firm.example%2Fterminal"
        )
    );
}

#[test]
fn this_computer_is_named_as_meridian_on_its_host() {
    let name = client_name();
    assert!(name.starts_with("meridian on "), "{name}");
    assert!(!name.chars().any(char::is_control), "{name}");
}

async fn visit(redirect_uri: &str, target: &str) -> String {
    let port = redirect_uri
        .trim_start_matches("http://127.0.0.1:")
        .trim_end_matches("/callback");
    let mut stream = tokio::net::TcpStream::connect(format!("127.0.0.1:{port}"))
        .await
        .expect("the listener is there");
    stream
        .write_all(format!("GET {target} HTTP/1.1\r\nhost: 127.0.0.1\r\n\r\n").as_bytes())
        .await
        .unwrap();
    let mut answer = String::new();
    stream.read_to_string(&mut answer).await.unwrap();
    answer
}

#[tokio::test]
async fn only_this_terminals_state_ends_the_wait() {
    let (listener, back) = listen().await.unwrap();
    let waiting =
        tokio::spawn(async move { returned(&listener, "mine", Duration::from_secs(10)).await });

    let favicon = visit(&back, "/favicon.ico").await;
    assert!(favicon.starts_with("HTTP/1.1 404"), "{favicon}");
    let theirs = visit(&back, "/callback?code=stolen&state=someone-elses").await;
    assert!(theirs.starts_with("HTTP/1.1 400"), "{theirs}");
    let ours = visit(&back, "/callback?code=the-code&state=mine").await;
    assert!(ours.contains("Connected"), "{ours}");

    assert_eq!(
        waiting.await.unwrap(),
        Ok(Returned::Code("the-code".into()))
    );
}

#[tokio::test]
async fn a_refusal_from_the_browser_ends_the_wait_without_a_code() {
    let (listener, back) = listen().await.unwrap();
    let waiting =
        tokio::spawn(async move { returned(&listener, "mine", Duration::from_secs(10)).await });
    visit(&back, "/callback?error=access_denied&state=mine").await;
    assert_eq!(waiting.await.unwrap(), Ok(Returned::Declined));
}

#[tokio::test]
async fn nobody_coming_back_ends_the_wait_with_a_reason() {
    let (listener, _) = listen().await.unwrap();
    let waited = returned(&listener, "mine", Duration::from_millis(200)).await;
    assert!(waited.unwrap_err().contains("nobody finished signing in"));
}

/// A deployment that answers one request with `status` and `body`, and
/// hands back what it was sent.
async fn deployment(
    status: &'static str,
    body: &'static str,
) -> (String, tokio::task::JoinHandle<String>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
    let served = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut buffer = vec![0u8; 16384];
        let mut read = 0;
        loop {
            let n = stream.read(&mut buffer[read..]).await.unwrap();
            read += n;
            let text = String::from_utf8_lossy(&buffer[..read]).to_string();
            if let Some((head, rest)) = text.split_once("\r\n\r\n") {
                let length = head
                    .lines()
                    .find_map(|line| {
                        line.to_ascii_lowercase()
                            .strip_prefix("content-length: ")
                            .map(|v| v.trim().parse::<usize>().unwrap())
                    })
                    .unwrap_or(0);
                if rest.len() >= length || n == 0 {
                    let response = format!(
                        "HTTP/1.1 {status}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                        body.len()
                    );
                    stream.write_all(response.as_bytes()).await.unwrap();
                    return text;
                }
            }
        }
    });
    (address, served)
}

const ISSUED: &str = r#"{"access_token":"mda_a","token_type":"Bearer","expires_in":600,"refresh_token":"mdr_r","subject":"local|ada","delegation_id":"d1","delegation_expires_at":"2026-12-25T00:00:00Z","delegation_expires_in":7776000}"#;

#[tokio::test]
async fn this_computer_registers_as_the_cli_with_its_loopback_address() {
    let (address, served) = deployment("201 Created", r#"{"client_id":"mdc_laptop"}"#).await;
    let id = register(&address, "http://127.0.0.1:53682/callback")
        .await
        .unwrap();
    assert_eq!(id, "mdc_laptop");
    let sent = served.await.unwrap();
    assert!(sent.starts_with("POST /oauth/register "), "{sent}");
    let body: serde_json::Value =
        serde_json::from_str(sent.split("\r\n\r\n").nth(1).unwrap()).unwrap();
    assert_eq!(body["software_id"], "meridian-cli");
    assert_eq!(body["token_endpoint_auth_method"], "none");
    assert_eq!(
        body["redirect_uris"],
        serde_json::json!(["http://127.0.0.1:53682/callback"])
    );
}

#[tokio::test]
async fn a_dashboard_from_before_delegations_is_said_as_that() {
    let (address, _) = deployment("404 Not Found", "").await;
    let refused = register(&address, "http://127.0.0.1:1/callback").await;
    assert!(refused
        .unwrap_err()
        .contains("does not take delegations yet"));
}

#[tokio::test]
async fn the_code_and_verifier_are_exchanged_for_a_pair_as_the_deployment_expects() {
    let (address, served) = deployment("200 OK", ISSUED).await;
    let issued = exchange(
        &address,
        "mdc_laptop",
        "the-code",
        VERIFIER,
        "http://127.0.0.1:53682/callback",
    )
    .await
    .unwrap();
    assert_eq!(issued.subject, "local|ada");
    let held = issued.held(&address, "mdc_laptop", 1_000);
    assert!(held.is_delegation());
    assert_eq!(held.access_expires_at_s, 1_600);
    assert_eq!(held.expires_at_s, 1_000 + 7_776_000);
    assert_eq!(held.expires_at, "2026-12-25T00:00:00Z");
    let sent = served.await.unwrap();
    assert!(sent.starts_with("POST /oauth/token "), "{sent}");
    let resource = format!("{address}/terminal")
        .replace(':', "%3A")
        .replace('/', "%2F");
    assert!(
        sent.ends_with(&format!(
            "grant_type=authorization_code&code=the-code&code_verifier={VERIFIER}\
             &redirect_uri=http%3A%2F%2F127.0.0.1%3A53682%2Fcallback&client_id=mdc_laptop\
             &resource={resource}"
        )),
        "{sent}"
    );
}

#[tokio::test]
async fn a_refused_refresh_names_why() {
    let (address, served) = deployment(
        "400 Bad Request",
        r#"{"error":"invalid_grant","error_description":"the delegation was revoked; connect again","reason":"revoked"}"#,
    )
    .await;
    let refused = refresh(&address, "mdc_laptop", "mdr_r").await.unwrap_err();
    assert_eq!(refused.reason, "revoked");
    assert!(refused.said.contains("was revoked"), "{}", refused.said);
    let sent = served.await.unwrap();
    assert!(
        sent.ends_with("grant_type=refresh_token&refresh_token=mdr_r&client_id=mdc_laptop"),
        "{sent}"
    );
}

#[tokio::test]
async fn a_refused_code_is_said_with_the_deployments_answer() {
    let (address, _) = deployment("400 Bad Request", r#"{"error":"invalid_grant"}"#).await;
    let refused = exchange(
        &address,
        "mdc_x",
        "x",
        VERIFIER,
        "http://127.0.0.1:1/callback",
    )
    .await;
    assert!(refused.unwrap_err().contains("invalid_grant"));
}

#[tokio::test]
async fn signing_out_revokes_the_delegation_by_its_refresh_token() {
    let (address, served) = deployment("200 OK", "{}").await;
    let held = crate::sessions::Held {
        address: address.clone(),
        client_id: "mdc_laptop".into(),
        access_token: "mda_a".into(),
        refresh_token: "mdr_r".into(),
        ..Default::default()
    };
    sign_out(&held).await.unwrap();
    let sent = served.await.unwrap();
    assert!(sent.starts_with("POST /oauth/revoke "), "{sent}");
    assert!(
        sent.ends_with("token=mdr_r&token_type_hint=refresh_token&client_id=mdc_laptop"),
        "{sent}"
    );
}

#[tokio::test]
async fn signing_out_an_older_clis_session_presents_it_as_a_bearer() {
    let (address, served) = deployment("204 No Content", "").await;
    let held = crate::sessions::Held {
        address: address.clone(),
        session: "s3cr3t".into(),
        ..Default::default()
    };
    sign_out(&held).await.unwrap();
    let sent = served.await.unwrap();
    assert!(sent.starts_with("POST /terminal/sign-out "), "{sent}");
    assert!(
        sent.to_ascii_lowercase()
            .contains("authorization: bearer s3cr3t"),
        "{sent}"
    );
}

#[test]
fn plain_http_is_for_this_machine_alone_including_names_under_localhost() {
    for local in [
        "http://127.0.0.1:8443",
        "http://localhost:8443",
        "http://meridian.localhost",
        "http://meridian-e2e.localhost",
    ] {
        assert!(address(local).is_ok(), "{local}");
    }
    for remote in [
        "http://meridian.firm.example",
        "http://localhost.firm.example",
        "http://evil-localhost",
    ] {
        assert!(address(remote).is_err(), "{remote}");
    }
}

#[test]
fn the_command_to_run_is_bare_for_the_local_install() {
    assert_eq!(
        command_for("https://meridian.localhost"),
        "meridian connect"
    );
    // The plain HTTP a cluster test serves is not the local install's.
    assert_eq!(
        command_for("http://meridian.localhost"),
        "meridian connect http://meridian.localhost"
    );
    assert_eq!(
        command_for("https://meridian.firm.example"),
        "meridian connect https://meridian.firm.example"
    );
}
