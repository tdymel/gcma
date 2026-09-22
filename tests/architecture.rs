//! Architecture rules (hexagonal / DDD), checked with archunit.
//!
//! domain      pure model and rules; depends on nothing of ours
//! application use cases and the ports they need; depends on the domain only
//! infra       leaf adapters implementing ports or reading/writing files
//! wiring      the composition of leaf adapters into one repository
//! cli         the driving adapter; may use everything

use archunit::{FileInfo, assert_passes, project_files, project_layers};

const MAX_NON_BLANK_LINES: usize = 450;

#[test]
fn layers_only_depend_inwards() {
    let rule = project_layers()
        .layer("domain")
        .defined_by("src/domain/**")
        .layer("application")
        .defined_by("src/application/**")
        .layer("infra")
        .defined_by("src/adapters/git_cli/**")
        .layer("infra")
        .defined_by("src/adapters/gix_store.rs")
        .layer("infra")
        .defined_by("src/adapters/config_file.rs")
        .layer("infra")
        .defined_by("src/adapters/plan_file.rs")
        .layer("infra")
        .defined_by("src/adapters/hook_installer.rs")
        .layer("infra")
        .defined_by("src/adapters/convert.rs")
        .layer("wiring")
        .defined_by("src/adapters/repository.rs")
        .layer("cli")
        .defined_by("src/adapters/cli/**")
        .where_layer("domain")
        .may_only_depend_on_layers(&[])
        .where_layer("application")
        .may_only_depend_on_layers(&["domain"])
        .where_layer("infra")
        .may_only_depend_on_layers(&["application", "domain"])
        .where_layer("wiring")
        .may_only_depend_on_layers(&["infra", "application", "domain"])
        .where_layer("cli")
        .may_only_depend_on_layers(&["wiring", "infra", "application", "domain"]);
    assert_passes!(rule);
}

#[test]
fn domain_is_layered_internally() {
    // error and text are leaves; settings on text and error; scheduling on settings;
    // history may use all of them.
    let rule = project_layers()
        .layer("error")
        .defined_by("src/domain/error.rs")
        .layer("text")
        .defined_by("src/domain/text/**")
        .layer("paths")
        .defined_by("src/domain/paths/**")
        .layer("settings")
        .defined_by("src/domain/settings/**")
        .layer("scheduling")
        .defined_by("src/domain/scheduling/**")
        .layer("history")
        .defined_by("src/domain/history/**")
        .where_layer("error")
        .may_only_depend_on_layers(&[])
        .where_layer("text")
        .may_only_depend_on_layers(&["error"])
        .where_layer("paths")
        .may_only_depend_on_layers(&["error"])
        .where_layer("settings")
        .may_only_depend_on_layers(&["paths", "text", "error"])
        .where_layer("scheduling")
        .may_only_depend_on_layers(&["settings", "error"])
        .where_layer("history")
        .may_only_depend_on_layers(&["scheduling", "settings", "paths", "text", "error"]);
    assert_passes!(rule);
}

#[test]
fn application_is_layered_internally() {
    // Ports are the base; the use cases build on them and must not reach into each other
    // except planning (shared by the rewrite and the push guard).
    let rule = project_layers()
        .layer("ports")
        .defined_by("src/application/ports.rs")
        .layer("planning")
        .defined_by("src/application/planning/**")
        .layer("rewrite")
        .defined_by("src/application/rewrite/**")
        .layer("llm")
        .defined_by("src/application/llm/**")
        .layer("push_guard")
        .defined_by("src/application/push_guard.rs")
        .where_layer("ports")
        .may_only_depend_on_layers(&[])
        .where_layer("planning")
        .may_only_depend_on_layers(&["ports"])
        .where_layer("rewrite")
        .may_only_depend_on_layers(&["planning", "ports"])
        .where_layer("llm")
        .may_only_depend_on_layers(&["ports"])
        .where_layer("push_guard")
        .may_only_depend_on_layers(&["planning", "rewrite", "ports"]);
    assert_passes!(rule);
}

#[test]
fn leaf_backends_do_not_know_each_other() {
    let rule = project_files()
        .in_path("src/adapters/git_cli/**")
        .should_not()
        .depend_on_files()
        .in_path("src/adapters/gix_store.rs");
    assert_passes!(rule);
    let rule = project_files()
        .in_path("src/adapters/gix_store.rs")
        .should_not()
        .depend_on_files()
        .in_path("src/adapters/git_cli/**");
    assert_passes!(rule);
}

#[test]
fn domain_uses_no_io_or_framework_crates() {
    for banned in ["clap", "gix", "serde_yaml", "serde_json", "tempfile"] {
        let rule = project_files()
            .in_path("src/domain/**")
            .should_not()
            .depend_on_external_modules()
            .matching(banned);
        assert_passes!(rule);
    }
}

#[test]
fn application_uses_no_adapter_crates() {
    for banned in ["clap", "gix", "serde_yaml"] {
        let rule = project_files()
            .in_path("src/application/**")
            .should_not()
            .depend_on_external_modules()
            .matching(banned);
        assert_passes!(rule);
    }
}

#[test]
fn no_dependency_cycles() {
    let rule = project_files().in_path("src/**").should().have_no_cycles();
    assert_passes!(rule);
}

#[test]
fn files_stay_small_enough_to_have_one_responsibility() {
    let rule = project_files().in_path("src/**").should().adhere_to(
        |file: &FileInfo| file.non_blank_line_count <= MAX_NON_BLANK_LINES,
        "contain at most 450 non-blank lines",
    );
    assert_passes!(rule);
}
