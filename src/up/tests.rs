use super::*;

fn install() -> Install {
    Install {
        release: "meridian".into(),
        namespace: "meridian".into(),
        chart: "oci://ghcr.io/open-meridian/charts/meridian-runtime".into(),
        chart_version: None,
        deployment_id: "dep-7".into(),
        enrolment_code: "ENROL-9XK2".into(),
        platform: None,
        image: None,
        values: vec![],
        timeout: "10m".into(),
    }
}

/// The wizard's own form, in the shape the dashboard renders it.
const PAGE: &str = "\
<h1>Set up this deployment</h1>\
<form method=\"post\">\
<label>Host<input name=\"db_host\" value=\"\" placeholder=\"postgres\"></label>\
<label>Port<input name=\"db_port\" value=\"5432\" placeholder=\"5432\"></label>\
<label>Serving password<input type=\"password\" name=\"db_serving_password\" autocomplete=\"off\"></label>\
<label>How people sign in<select name=\"backend\"><option value=\"local\">An account here</option></select></label>\
<button formaction=\"/first-run/check\">Test</button>\
<button formaction=\"/first-run/apply\">Apply</button>\
</form>";

#[test]
fn the_enrolment_code_never_reaches_an_argument() {
    let arguments = helm_arguments(&install()).join(" ");

    assert!(
        !arguments.contains("ENROL-9XK2"),
        "arguments are readable by every process on the machine: {arguments}"
    );
    assert!(arguments.contains("--values -"), "{arguments}");
    assert!(values_document(&install()).contains("ENROL-9XK2"));
}

#[test]
fn an_install_that_was_told_no_platform_writes_none() {
    // The chart holds the address, and an install that wrote it down would be
    // carrying a value nobody gave it -- which is how a values file ends up
    // pinning a platform somebody later moves.
    let document = values_document(&install());

    assert!(!document.contains("platform"), "{document}");
}

#[test]
fn a_platform_somebody_named_is_carried() {
    // A staging platform, while somebody tests a change to the platform.
    let mut intended = install();
    intended.platform = Some("https://uat.open-meridian.com".into());

    assert!(values_document(&intended).contains("uat.open-meridian.com"));
}

#[test]
fn what_is_printed_holds_no_credential() {
    let said = shown(&install());

    assert!(!said.contains("ENROL-9XK2"), "{said}");
    assert!(said.contains("$MERIDIAN_ENROLMENT_CODE"), "{said}");
    // And it is still the command somebody could run.
    assert!(
        said.starts_with("helm upgrade --install meridian "),
        "{said}"
    );
}

#[test]
fn a_persons_own_values_come_before_this_commands_own() {
    let mut intended = install();
    intended.values = vec!["mine.yaml".into()];

    let arguments = helm_arguments(&intended);
    let mine = arguments.iter().position(|a| a == "mine.yaml").unwrap();
    let ours = arguments.iter().rposition(|a| a == "-").unwrap();

    assert!(
        mine < ours,
        "what was typed wins over what a file holds: {arguments:?}"
    );
}

#[test]
fn nothing_waits_for_the_whole_release() {
    // A fresh install has no database and no directory. Waiting for the
    // components that need them is waiting for answers nobody has given.
    assert!(!helm_arguments(&install()).iter().any(|a| a == "--wait"));
}

#[test]
fn an_image_override_becomes_a_repository_and_a_tag() {
    let mut intended = install();
    intended.image = Some("registry.firm.internal/meridian:2026-09-23".into());

    let document = values_document(&intended);

    assert!(
        document.contains("repository: \"registry.firm.internal/meridian\""),
        "{document}"
    );
    assert!(document.contains("tag: \"2026-09-23\""), "{document}");
}

#[test]
fn a_params_file_holding_a_password_is_refused_with_where_to_put_it() {
    let refusal = params_from("db_host: postgres\ndb_serving_password: hunter2\n")
        .expect_err("a file with a password in it");

    assert!(
        refusal.contains("MERIDIAN_DB_SERVING_PASSWORD"),
        "{refusal}"
    );
    assert!(refusal.contains("gets committed"), "{refusal}");
}

#[test]
fn a_client_secret_is_a_credential_too() {
    assert!(params_from("oidc_client_secret: abc\n").is_err());
    assert!(params_from("ldap_bind_passphrase: abc\n").is_err());
}

#[test]
fn numbers_and_flags_are_answers_like_any_other() {
    let params = params_from("db_port: 5432\ndb_tls: true\ndb_host: postgres\n").unwrap();

    assert_eq!(params["db_port"], "5432");
    assert_eq!(params["db_tls"], "true");
}

#[test]
fn a_nested_document_is_answering_a_shape_the_form_does_not_have() {
    let refusal = params_from("database:\n  host: postgres\n").expect_err("nested");
    assert!(refusal.contains("scalar"), "{refusal}");
}

#[test]
fn the_form_itself_says_which_fields_are_credentials() {
    let (fields, credentials) = asked_for(PAGE);

    assert!(
        fields.contains("db_host") && fields.contains("backend"),
        "{fields:?}"
    );
    assert_eq!(
        credentials.iter().cloned().collect::<Vec<_>>(),
        vec!["db_serving_password".to_string()]
    );
    // A button is not a field.
    assert!(!fields.contains("Test"));
}

#[test]
fn a_typo_is_refused_against_the_form_rather_than_posted_as_nothing() {
    let params = params_from("db_hostname: postgres\n").unwrap();
    let (fields, credentials) = asked_for(PAGE);

    let refusals = answers(&params, &fields, &credentials, &|_| None).expect_err("a typo");

    assert_eq!(refusals.len(), 1);
    assert!(refusals[0].contains("db_host"), "{refusals:?}");
}

#[test]
fn credentials_come_from_the_environment_and_the_rest_from_the_file() {
    let params = params_from("db_host: postgres\ndb_port: 5432\nbackend: local\n").unwrap();
    let (fields, credentials) = asked_for(PAGE);

    let posted = answers(&params, &fields, &credentials, &|named| {
        (named == "MERIDIAN_DB_SERVING_PASSWORD").then(|| "hunter2".to_string())
    })
    .unwrap();

    assert_eq!(posted["db_host"], "postgres");
    assert_eq!(posted["db_serving_password"], "hunter2");
}

#[test]
fn a_credential_the_environment_does_not_hold_is_left_to_the_wizard_to_refuse() {
    // Choosing one way to sign in leaves the other two's fields empty, and an
    // empty credential for a route nobody chose is correct.
    let params = params_from("db_host: postgres\n").unwrap();
    let (fields, credentials) = asked_for(PAGE);

    let posted = answers(&params, &fields, &credentials, &|_| None).unwrap();

    assert!(!posted.contains_key("db_serving_password"), "{posted:?}");
}

#[test]
fn the_wizards_findings_are_read_back_as_it_lists_them() {
    let page = "<ul class=\"refusal\"><li>the serving role may create tables</li>\
                <li>this deployment could not sign in to the directory</li></ul>";

    assert_eq!(
        findings(page),
        vec![
            "the serving role may create tables".to_string(),
            "this deployment could not sign in to the directory".to_string(),
        ]
    );
    assert!(!passes(page));
}

#[test]
fn passing_is_the_class_the_wizard_marks_it_with() {
    assert!(passes("<p class=\"passed\">Everything passes.</p>"));
}

#[test]
fn a_lost_session_is_the_page_that_asks_for_a_code() {
    assert!(wants_a_code(
        "<form method=\"post\" action=\"/first-run/claim\">"
    ));
    assert!(
        !wants_a_code(PAGE),
        "the wizard itself does not ask for a code"
    );
}

#[test]
fn who_administers_it_is_read_off_the_page_that_says_so() {
    // The wizard's applied page, as meridian-core's dashboard writes it for
    // each way of naming an administrator.
    let local = "<h1>This deployment is configured</h1>\
                 <p><strong>ada</strong> administers this deployment. Sign in with \
                 the account and password you just gave; there is nothing to redeem.</p>";
    let group = "<p>Everybody in <strong>meridian-admins</strong> administers this \
                 deployment. Sign in through the directory you configured.</p>";

    assert_eq!(
        administrator(local).as_deref(),
        Some("ada administers this deployment")
    );
    assert_eq!(
        administrator(group).as_deref(),
        Some("Everybody in meridian-admins administers this deployment")
    );
    // A page that did not finish says nothing of the kind.
    assert_eq!(
        administrator("<ul class=\"refusal\"><li>nope</li></ul>"),
        None
    );
}

#[test]
fn a_form_post_escapes_what_a_password_can_hold() {
    // A password is a string, and a string with `&` in it would otherwise end
    // one field and start another.
    let fields = BTreeMap::from([
        ("db_host".to_string(), "postgres.internal".to_string()),
        ("db_serving_password".to_string(), "a&b=c d+e%".to_string()),
    ]);

    assert_eq!(
        form_encoded(&fields),
        "db_host=postgres.internal&db_serving_password=a%26b%3Dc%20d%2Be%25"
    );
}
