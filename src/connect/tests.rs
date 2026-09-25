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
fn the_sign_in_address_carries_the_challenge_and_an_encoded_callback() {
    let pkce = Pkce {
        verifier: VERIFIER.into(),
        challenge: CHALLENGE.into(),
    };
    assert_eq!(
        authorize_url(
            "https://dash.firm.example",
            "http://127.0.0.1:53682/callback",
            &pkce,
            "st-1"
        ),
        format!(
            "https://dash.firm.example/terminal/authorize?\
             redirect_uri=http%3A%2F%2F127.0.0.1%3A53682%2Fcallback&code_challenge={CHALLENGE}\
             &code_challenge_method=S256&state=st-1"
        )
    );
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

#[tokio::test]
async fn the_code_and_verifier_are_exchanged_as_the_deployment_expects() {
    let (address, served) = deployment(
        "200 OK",
        r#"{"session":"s3cr3t","subject":"local|ada","idle_seconds":1800,"expires_at":"2026-09-26T12:00:00Z"}"#,
    )
    .await;
    let issued = exchange(
        &address,
        "the-code",
        VERIFIER,
        "http://127.0.0.1:53682/callback",
    )
    .await
    .unwrap();
    assert_eq!(issued.session, "s3cr3t");
    assert_eq!(issued.subject, "local|ada");
    let sent = served.await.unwrap();
    assert!(sent.starts_with("POST /terminal/token "), "{sent}");
    assert!(
        sent.ends_with(&format!(
            "code=the-code&code_verifier={VERIFIER}&redirect_uri=http%3A%2F%2F127.0.0.1%3A53682%2Fcallback"
        )),
        "{sent}"
    );
}

#[tokio::test]
async fn a_refused_code_is_said_with_the_deployments_answer() {
    let (address, _) = deployment("400 Bad Request", r#"{"error":"invalid_grant"}"#).await;
    let refused = exchange(&address, "x", VERIFIER, "http://127.0.0.1:1/callback").await;
    assert!(refused.unwrap_err().contains("invalid_grant"));
}

#[tokio::test]
async fn signing_out_presents_the_session_as_a_bearer() {
    let (address, served) = deployment("204 No Content", "").await;
    sign_out(&address, "s3cr3t").await.unwrap();
    let sent = served.await.unwrap();
    assert!(sent.starts_with("POST /terminal/sign-out "), "{sent}");
    assert!(
        sent.to_ascii_lowercase()
            .contains("authorization: bearer s3cr3t"),
        "{sent}"
    );
}
