use super::*;

#[test]
fn where_it_was_reached_is_its_ingress_name_when_it_had_one() {
    let local = r#"{"deployment":{"id":"DEP-X"},"ingress":{"enabled":true,"host":"meridian.localhost","className":"traefik"}}"#;
    assert_eq!(
        reached_at(local).as_deref(),
        Some("http://meridian.localhost")
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
