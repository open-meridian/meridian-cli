use super::*;

fn asked(delete_namespace: bool) -> Down {
    Down {
        release: "meridian".into(),
        namespace: "meridian".into(),
        delete_namespace,
    }
}

#[test]
fn the_questions_say_what_goes() {
    let uninstall = uninstall_question(&asked(false));
    assert!(uninstall.contains("The namespace is kept"), "{uninstall}");
    let namespace = namespace_question(&asked(true));
    for said in ["database", "nothing backs up", "private key", "revoked"] {
        assert!(namespace.contains(said), "{said}: {namespace}");
    }
}

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
