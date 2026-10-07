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
        ingress: None,
        development: false,
        tls_secret: None,
        plain_http: false,
        archive: None,
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

#[test]
fn the_ingress_is_asked_for_by_name_and_class_and_otherwise_not_at_all() {
    assert!(!values_document(&install()).contains("ingress"));
    let through = Install {
        ingress: Some(Ingress {
            host: "meridian.localhost".into(),
            class: "traefik".into(),
        }),
        ..install()
    };
    let values = values_document(&through);
    assert!(
        values.contains(
            "ingress:\n  enabled: true\n  host: \"meridian.localhost\"\n  className: \"traefik\"\n"
        ),
        "{values}"
    );
}

#[test]
fn the_class_is_the_default_or_the_only_one_and_never_a_guess() {
    assert_eq!(chosen_class("traefik\t\n").as_deref(), Some("traefik"));
    assert_eq!(
        chosen_class("nginx\t\ntraefik\ttrue\n").as_deref(),
        Some("traefik")
    );
    assert_eq!(chosen_class("nginx\t\ntraefik\t\n"), None);
    assert_eq!(chosen_class(""), None);
}

#[test]
fn development_is_asked_for_only_when_said() {
    assert!(!values_document(&install()).contains("development"));
    let marked = Install {
        development: true,
        ..install()
    };
    assert!(values_document(&marked).contains("development: true\n"));
}

#[test]
fn a_name_with_a_certificate_is_reached_over_https_and_only_a_local_one_without() {
    // A development deployment's local name has this machine's certificate.
    assert_eq!(
        address_of("meridian.localhost", true),
        "https://meridian.localhost"
    );
    // Without one, plain HTTP, which the chart serves for the cluster tests.
    assert_eq!(
        address_of("meridian.localhost", false),
        "http://meridian.localhost"
    );
    assert_eq!(address_of("localhost", false), "http://localhost");
    // Any other name is HTTPS, with a certificate or with none.
    for tls in [true, false] {
        assert_eq!(
            address_of("meridian.firm.example", tls),
            "https://meridian.firm.example"
        );
        // Not a suffix match on the text: `notlocalhost` is somebody's domain.
        assert_eq!(address_of("notlocalhost", tls), "https://notlocalhost");
    }
}

#[test]
fn a_local_certificate_and_plain_http_are_values_only_when_said() {
    let through = Install {
        ingress: Some(Ingress {
            host: "meridian.localhost".into(),
            class: "traefik".into(),
        }),
        ..install()
    };
    let values = values_document(&through);
    assert!(
        !values.contains("tls") && !values.contains("plainHttp"),
        "{values}"
    );
    let served = Install {
        tls_secret: Some("meridian-tls".into()),
        ..through.clone()
    };
    assert!(values_document(&served)
        .ends_with("className: \"traefik\"\n  tls:\n    secretName: \"meridian-tls\"\n"));
    let tested = Install {
        plain_http: true,
        ..through
    };
    assert!(values_document(&tested).contains("  plainHttp: true\n"));
}

#[test]
fn an_id_is_the_platforms_shape_or_nothing_is_installed() {
    assert_eq!(check_id("DEP-01M3GZ8K4Q7T2V9W6X5Y3R1N0P"), Ok(()));
    let doubled = check_id("DEP-DEP-01M3GZ8K4Q7T2V9W6X5Y3R1N0P").unwrap_err();
    assert!(doubled.contains("DEP- twice"), "{doubled}");
    assert!(
        doubled.contains("`DEP-01M3GZ8K4Q7T2V9W6X5Y3R1N0P`"),
        "{doubled}"
    );
    for wrong in [
        "dep-7",
        "01M3GZ8K4Q7T2V9W6X5Y3R1N0P",
        "DEP-XXXX-XXXX",
        "DEP-01M3GZ8K4Q7T2V9W6X5Y3R1N0",
        "DEP-01m3gz8k4q7t2v9w6x5y3r1n0p",
        "DEP-01M3GZ8K4Q7T2V9W6X5Y3R1N0L",
    ] {
        assert!(check_id(wrong).is_err(), "{wrong}");
    }
}

// ── The archive (contract v16; W7.1) ─────────────────────────────────────────

#[test]
fn an_archive_is_the_charts_plugin_archive_path_and_none_writes_nothing() {
    for none in [None, Some(Archive::None)] {
        let declined = Install {
            archive: none,
            ..install()
        };
        assert!(!values_document(&declined).contains("pluginArchive"));
    }
    let named = Install {
        archive: Some(Archive::At("/mnt/nas/meridian-archive".into())),
        ..install()
    };
    assert!(
        values_document(&named).contains("pluginArchive:\n  path: \"/mnt/nas/meridian-archive\"\n")
    );
    // A values file's archive is the file's: nothing written over it.
    let in_values = Install {
        archive: Some(Archive::InValues("cloud.yaml".into())),
        ..install()
    };
    assert!(!values_document(&in_values).contains("pluginArchive"));
}

#[test]
fn the_flags_name_an_archive_or_decline_one_and_never_both() {
    assert_eq!(archive_from_flags(None, false), Ok(None));
    assert_eq!(archive_from_flags(None, true), Ok(Some(Archive::None)));
    assert_eq!(
        archive_from_flags(Some("/mnt/nas/archive"), false),
        Ok(Some(Archive::At("/mnt/nas/archive".into())))
    );
    let both = archive_from_flags(Some("/mnt/nas/archive"), true).unwrap_err();
    assert!(both.contains("say one"), "{both}");
    for wrong in ["archive", "./archive", "/", "/mnt/../etc", "~/archive"] {
        let refused = archive_from_flags(Some(wrong), false).unwrap_err();
        assert!(refused.contains("absolute path"), "{wrong}: {refused}");
    }
}

#[test]
fn the_archive_is_said_in_one_place_or_asked() {
    // Nothing said: `up` asks.
    assert_eq!(archive_settled(None, None, None), Ok(None));
    // The params file answers as a person would at the question.
    assert_eq!(
        archive_settled(None, Some("/srv/archive"), None),
        Ok(Some(Archive::At("/srv/archive".into())))
    );
    for none in ["", "none", "false"] {
        assert_eq!(
            archive_settled(None, Some(none), None),
            Ok(Some(Archive::None))
        );
    }
    assert!(archive_settled(None, Some("archive"), None).is_err());
    // A values file naming one, as a cloud's bucket is: not asked.
    assert_eq!(
        archive_settled(None, None, Some("cloud.yaml")),
        Ok(Some(Archive::InValues("cloud.yaml".into())))
    );
    // Two places: refused, naming both, so neither is quietly dropped.
    let twice = archive_settled(Some(Archive::None), Some("/srv/archive"), None).unwrap_err();
    assert!(
        twice.contains("the command line") && twice.contains("params file"),
        "{twice}"
    );
    let twice = archive_settled(None, Some("none"), Some("cloud.yaml")).unwrap_err();
    assert!(twice.contains("cloud.yaml"), "{twice}");
}

#[test]
fn a_values_file_names_an_archive_by_a_path_a_claim_or_a_bucket() {
    assert!(!names_an_archive("image:\n  tag: x\n"));
    assert!(!names_an_archive(
        "pluginArchive:\n  path: \"\"\n  size: 1Ti\n"
    ));
    assert!(names_an_archive("pluginArchive:\n  path: /mnt/nas\n"));
    assert!(names_an_archive("pluginArchive:\n  existingClaim: nas\n"));
    assert!(names_an_archive(
        "pluginArchive:\n  bucket: s3://firm-archive\n  serviceAccount: archive\n"
    ));
    assert!(!names_an_archive("not: [yaml"));
}

#[test]
fn a_params_files_archive_is_ups_own_answer_and_no_field_of_the_wizards() {
    let params = params_from("db_host: postgres\narchive: /srv/archive\n").unwrap();
    assert_eq!(
        params.get("archive").map(String::as_str),
        Some("/srv/archive")
    );
    // run::up takes it out before the wizard's fields are matched, or the
    // wizard would refuse a field it does not ask for.
    let (fields, credentials) = asked_for(PAGE);
    let refused = answers(&params, &fields, &credentials, &|_| None).unwrap_err();
    assert!(refused.iter().any(|said| said.contains("`archive`")));
}
