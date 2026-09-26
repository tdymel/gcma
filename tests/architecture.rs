//! Architecture rules (hexagonal / DDD), checked with archunit.
//!
//! domain      pure model and rules; depends on nothing of ours
//! application use cases and the ports they need; depends on the domain only
//! infra       leaf adapters implementing ports or reading/writing files
//! wiring      the composition of leaf adapters into one repository
//! cli         the driving adapter (the commands and their shared support); may use everything

use archunit::{FileInfo, assert_passes, project_files, project_layers};

const MAX_NON_BLANK_LINES: usize = 350;

/// Every place under `src/` that belongs to a layer: (layer, glob). The layer rules and the check
/// that no adapter file escapes them are both derived from this one list, so they cannot drift.
/// archunit ignores an edge with an unassigned end, so a file missing here is not checked at all.
const PLACES: &[(&str, &str)] = &[
    ("domain", "src/domain/**"),
    ("application", "src/application/**"),
    ("infra", "src/adapters/git_cli/**"),
    ("infra", "src/adapters/gix_store.rs"),
    ("infra", "src/adapters/config_file.rs"),
    ("infra", "src/adapters/plan_file.rs"),
    ("infra", "src/adapters/hook_installer.rs"),
    ("infra", "src/adapters/llm_jsonl.rs"),
    ("infra", "src/adapters/convert.rs"),
    ("infra", "src/adapters/fsutil.rs"),
    ("infra", "src/adapters/fault_injection.rs"),
    ("wiring", "src/adapters/repository.rs"),
    ("cli", "src/adapters/mod.rs"),
    ("cli", "src/adapters/cli/**"),
    ("cli", "src/adapters/cli_support/**"),
];

#[test]
fn layers_only_depend_inwards() {
    let layers = PLACES
        .iter()
        .fold(project_layers(), |layers, (layer, glob)| {
            layers.layer(*layer).defined_by(*glob)
        });
    let rule = layers
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
        .layer("preconditions")
        .defined_by("src/application/preconditions.rs")
        .layer("pathrules")
        .defined_by("src/application/pathrules.rs")
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
        .where_layer("pathrules")
        .may_only_depend_on_layers(&["ports"])
        .where_layer("preconditions")
        .may_only_depend_on_layers(&["ports"])
        .where_layer("planning")
        .may_only_depend_on_layers(&["preconditions", "pathrules", "ports"])
        .where_layer("rewrite")
        .may_only_depend_on_layers(&["preconditions", "pathrules", "planning", "ports"])
        .where_layer("llm")
        .may_only_depend_on_layers(&["ports"])
        .where_layer("push_guard")
        .may_only_depend_on_layers(&["planning", "rewrite", "ports"]);
    assert_passes!(rule);
}

/// Whether `path` is covered by a glob of `PLACES`: a `dir/**` prefix or an exact file.
fn is_placed(path: &str) -> bool {
    PLACES
        .iter()
        .any(|(_, glob)| match glob.strip_suffix("**") {
            Some(dir) => path.starts_with(dir),
            None => path == *glob,
        })
}

#[test]
fn every_adapter_file_is_assigned_to_a_layer() {
    let rule = project_files()
        .in_path("src/adapters/**")
        .should()
        .adhere_to(
            |file: &FileInfo| is_placed(&file.path),
            "be assigned to a layer in PLACES",
        );
    assert_passes!(rule);
}

/// archunit sees `std` only as a whole, so I/O and the clock are checked on the source text.
const FORBIDDEN_IN_INNER_LAYERS: &[&str] = &[
    "std::fs",
    "std::process",
    "std::env",
    "fs_err",
    // A dev-dependency, which archunit does not see as an external module.
    "tempfile",
    "Utc::now",
    "Local::now",
    "SystemTime::now",
    "Instant::now",
    "serde_yaml",
    "serde_yaml_ng",
    "clap::",
];

/// Items that must not be named inside a grouped `std::{...}` import, which the plain text
/// patterns above cannot see.
const FORBIDDEN_STD_ITEMS: &[&str] = &["fs", "process", "env", "SystemTime", "Instant"];

/// The body of every `std::{ ... }` group in `source`, however many lines it spans or how deeply
/// it nests.
fn std_groups(source: &str) -> Vec<&str> {
    let mut groups = Vec::new();
    let mut rest = source;
    while let Some(at) = rest.find("std::{") {
        let body = &rest[at + "std::{".len()..];
        let mut depth = 1;
        let end = body
            .char_indices()
            .find(|&(_, c)| {
                match c {
                    '{' => depth += 1,
                    '}' => depth -= 1,
                    _ => {}
                }
                depth == 0
            })
            .map_or(body.len(), |(i, _)| i);
        groups.push(&body[..end]);
        rest = &body[end..];
    }
    groups
}

fn touches_io_or_the_clock(source: &str) -> bool {
    FORBIDDEN_IN_INNER_LAYERS
        .iter()
        .any(|bad| source.contains(bad))
        || std_groups(source).iter().any(|group| {
            group
                .split(|c: char| !(c.is_alphanumeric() || c == '_'))
                .any(|word| FORBIDDEN_STD_ITEMS.contains(&word))
        })
}

#[test]
fn the_source_scan_sees_grouped_imports() {
    for bad in [
        "use std::fs;",
        "use std::{fs, env};",
        "use std::{io, path::Path, process::Command};",
        "use std::{\n    collections::HashMap,\n    env,\n};",
        "use std::{io::{self, Read}, time::Instant};",
        "use fs_err as fs;",
    ] {
        assert!(touches_io_or_the_clock(bad), "should flag {bad:?}");
    }
    for good in [
        "use std::{collections::HashMap, path::Path};",
        "use std::{fmt, io::{self, Read}};\nfn fs_like() {}",
        "use std::fmt;",
    ] {
        assert!(!touches_io_or_the_clock(good), "should not flag {good:?}");
    }
}

#[test]
fn domain_and_application_do_no_io_and_read_no_clock() {
    for scope in ["src/domain/**", "src/application/**"] {
        let rule = project_files().in_path(scope).should().adhere_to(
            |file: &FileInfo| !touches_io_or_the_clock(&file.content),
            "not touch the file system, processes, the environment, the clock or the CLI",
        );
        assert_passes!(rule);
    }
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
    for banned in [
        "clap",
        "gix",
        "serde_yaml",
        "serde_yaml_ng",
        "serde_json",
        "tempfile",
        "fs_err",
    ] {
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
    for banned in [
        "clap",
        "gix",
        "serde_yaml",
        "serde_yaml_ng",
        "serde_json",
        "fs_err",
        "tempfile",
    ] {
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
        "contain at most 350 non-blank lines",
    );
    assert_passes!(rule);
}
