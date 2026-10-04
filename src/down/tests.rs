use super::*;

#[test]
fn where_it_was_reached_is_its_ingress_name_when_it_had_one() {
    let local = r#"{"deployment":{"id":"DEP-X"},"ingress":{"enabled":true,"host":"meridian.localhost","className":"traefik"}}"#;
    assert_eq!(
        reached_at(local).as_deref(),
        Some("http://meridian.localhost")
    );
    let served = r#"{"ingress":{"enabled":true,"host":"meridian.localhost","tls":{"secretName":"meridian-tls"}}}"#;
    assert_eq!(
        reached_at(served).as_deref(),
        Some("https://meridian.localhost")
    );
    let firm = r#"{"ingress":{"enabled":true,"host":"meridian.firm.example"}}"#;
    assert_eq!(
        reached_at(firm).as_deref(),
        Some("https://meridian.firm.example")
    );
    for none in [
        r#"{"deployment":{"id":"DEP-X"}}"#,
        r#"{"ingress":{"enabled":false,"host":"meridian.localhost"}}"#,
        r#"{"ingress":{"enabled":true,"host":""}}"#,
        "null",
        "not json",
    ] {
        assert_eq!(reached_at(none), None, "{none}");
    }
}

#[test]
fn the_certificate_up_wrote_is_its_own_secret_and_no_other() {
    let served = r#"{"ingress":{"tls":{"secretName":"meridian-tls"}}}"#;
    assert!(wrote_a_certificate(served, "meridian"));
    assert!(!wrote_a_certificate(served, "trial"));
    let firms = r#"{"ingress":{"tls":{"secretName":"dash-tls"}}}"#;
    assert!(!wrote_a_certificate(firms, "meridian"));
    assert!(!wrote_a_certificate("{}", "meridian"));
}
