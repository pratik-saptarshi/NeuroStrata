//! Documentation Manifest & Surface-Diff Gate.
//!
//! Consumes the pluggable workflow connector for the changed-surface set and
//! evaluates the 3-tier Documentation Manifest (`docs/manifest.json`).

use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DocTierSpec {
    pub dir: String,
    pub description: String,
    #[serde(default)]
    pub required_files: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DocTiers {
    pub vision: DocTierSpec,
    pub features_api: DocTierSpec,
    pub extend: DocTierSpec,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SurfaceDocRule {
    pub id: String,
    pub tier: String,
    #[serde(default)]
    pub watch_paths: Vec<String>,
    pub target_doc: String,
    #[serde(default)]
    pub required_headings: Vec<String>,
    #[serde(default)]
    pub required_tokens: Vec<String>,
    #[serde(default)]
    pub require_co_change_on_surface_change: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DocManifest {
    pub schema_version: u32,
    #[serde(default)]
    pub allowed_root_files: Vec<String>,
    pub tiers: DocTiers,
    #[serde(default)]
    pub support_dirs: Vec<String>,
    #[serde(default)]
    pub prohibited_filename_substrings: Vec<String>,
    #[serde(default)]
    pub surface_rules: Vec<SurfaceDocRule>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DocViolation {
    pub rule_id: String,
    pub kind: String,
    pub path: String,
    pub detail: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DocGateReport {
    pub passed: bool,
    pub connector_id: String,
    pub checked_tiers: usize,
    pub checked_rules: usize,
    pub changed_paths: Vec<String>,
    pub violations: Vec<DocViolation>,
}

pub fn load_manifest(root: &Path) -> Result<DocManifest> {
    let manifest_path = root.join("docs").join("manifest.json");
    let json = std::fs::read_to_string(&manifest_path)?;
    Ok(serde_json::from_str(&json)?)
}

fn collect_files_recursive(dir: &Path, out: &mut Vec<std::path::PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let p = entry.path();
        if p.is_dir() {
            collect_files_recursive(&p, out);
        } else if p.is_file() {
            out.push(p);
        }
    }
}

/// Markdown files under `docs/` (including `support_dirs`) subject to the
/// machine-path content rule.
///
/// Inside a git worktree (`root.join(".git").exists()`), only git-tracked
/// markdown files under `docs/` are scanned so an untracked local scratch file
/// never blocks the gate. Outside a git worktree (temp-dir unit tests), every
/// markdown file under `docs/` is scanned.
fn collect_doc_markdown_files(root: &Path) -> Vec<std::path::PathBuf> {
    let tracked: Option<std::collections::BTreeSet<String>> = if root.join(".git").exists() {
        std::process::Command::new("git")
            .args(["ls-files", "--", "docs"])
            .current_dir(root)
            .output()
            .ok()
            .filter(|out| out.status.success())
            .map(|out| {
                String::from_utf8_lossy(&out.stdout)
                    .lines()
                    .map(|line| line.trim().replace('\\', "/"))
                    .filter(|line| !line.is_empty())
                    .collect()
            })
    } else {
        None
    };

    let mut files = Vec::new();
    collect_files_recursive(&root.join("docs"), &mut files);
    files
        .into_iter()
        .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("md"))
        .filter(|p| {
            let Some(tracked) = &tracked else {
                return true;
            };
            let Ok(rel) = p.strip_prefix(root) else {
                return false;
            };
            let rel = rel.to_string_lossy().replace('\\', "/");
            tracked.contains(&rel)
        })
        .collect()
}

/// Extracts public surface identifiers (CLI `#[command(name = "...")]` subcommands,
/// MCP `"name": "neurostrata_..."` tool identifiers, and `pub trait <Name>` declarations)
/// from added diff lines or source text.
pub fn extract_public_surface_identifiers(text: &str) -> Vec<String> {
    let mut idents = std::collections::BTreeSet::new();
    for line in text.lines() {
        let trimmed = line.trim();
        if let Some(idx) = trimmed.find("#[command(name = \"") {
            let rest = &trimmed[idx + "#[command(name = \"".len()..];
            if let Some(end) = rest.find('"') {
                let name = &rest[..end];
                if !name.is_empty() && name != "neurostrata-mcp" {
                    idents.insert(name.to_string());
                }
            }
        }
        if let Some(idx) = trimmed.find("\"name\": \"neurostrata_") {
            let rest = &trimmed[idx + "\"name\": \"".len()..];
            if let Some(end) = rest.find('"') {
                let name = &rest[..end];
                if !name.is_empty() {
                    idents.insert(name.to_string());
                }
            }
        }
        if let Some(idx) = trimmed.find("pub trait ") {
            let rest = &trimmed[idx + "pub trait ".len()..];
            let ident: String = rest
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
                .collect();
            if !ident.is_empty() && ident != "AsAny" {
                idents.insert(ident);
            }
        }
    }
    idents.into_iter().collect()
}

fn rename_source(root: &Path, range: &str, rel_path: &str) -> Option<String> {
    let out = std::process::Command::new("git")
        .args(["diff", "-M", "--name-status", "--diff-filter=R", range])
        .current_dir(root)
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .find_map(|line| {
            let fields: Vec<_> = line.split('\t').collect();
            (fields.len() == 3 && fields[2] == rel_path).then(|| fields[1].to_string())
        })
}

/// Parses one line as a public item declaration, returning its
/// (declared name, item kind) when the line begins a `pub` / `pub(...)`
/// item (with optional async / unsafe / const / default qualifiers) of a
/// known kind. For `use`, the declared name is the final path segment,
/// honouring `as` renames.
fn parse_public_surface_declaration(line: &str) -> Option<(String, String)> {
    const ITEM_KINDS: [&str; 10] = [
        "struct", "enum", "trait", "fn", "type", "const", "static", "mod", "use", "union",
    ];
    {
        let trimmed = line.trim();
        let rest = trimmed.strip_prefix("pub")?;
        // `pub` must stand alone (bare) or open a visibility group `pub(...)`.
        let rest = if let Some(after_paren) = rest.strip_prefix('(') {
            let Some(close) = after_paren.find(')') else {
                return None;
            };
            &after_paren[close + 1..]
        } else if rest.starts_with(char::is_whitespace) {
            rest
        } else {
            return None;
        };
        let mut tokens: Vec<&str> = rest.split_whitespace().collect();
        // Optional qualifiers; `const` is a qualifier only before `fn`
        // (otherwise it is the item kind itself, e.g. `pub const NAME`).
        let mut start = 0;
        while start < tokens.len() {
            match tokens[start] {
                "async" | "unsafe" | "default" => start += 1,
                "const" if tokens.get(start + 1) == Some(&"fn") => start += 1,
                _ => break,
            }
        }
        tokens.drain(..start);
        let Some((kind, _rest)) = tokens.split_first() else {
            return None;
        };
        if !ITEM_KINDS.contains(kind) {
            return None;
        }
        let name_token = tokens.get(1);
        let name: String = if *kind == "use" {
            let path = tokens[1..].join(" ");
            let path = path.trim().trim_end_matches(';').trim();
            if let Some((_, alias)) = path.rsplit_once(" as ") {
                alias
                    .chars()
                    .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
                    .collect()
            } else {
                path.rsplit("::")
                    .next()
                    .unwrap_or_default()
                    .trim()
                    .trim_end_matches([';', '}', ','])
                    .chars()
                    .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
                    .collect()
            }
        } else {
            let Some(name_token) = name_token else {
                return None;
            };
            name_token
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
                .collect()
        };
        if name.is_empty() {
            return None;
        }
        Some((name, (*kind).to_string()))
    }
}

/// Extracts public surface declarations from source text or diff lines:
/// everything `extract_public_surface_identifiers` returns, plus public item
/// declarations (`pub` / `pub(...)` items, with optional async / unsafe /
/// const / default qualifiers, over struct/enum/trait/fn/type/const/static/
/// mod/use/union). For `use`, the declared name is the final path segment,
/// honouring `as` renames.
pub fn extract_public_surface_declarations(text: &str) -> std::collections::BTreeSet<String> {
    let mut decls: std::collections::BTreeSet<String> = extract_public_surface_identifiers(text)
        .into_iter()
        .collect();
    for line in text.lines() {
        if let Some((name, _kind)) = parse_public_surface_declaration(line) {
            decls.insert(name);
        }
    }
    decls
}

/// Normalizes a declaration line into its public signature: whitespace
/// runs collapse to one space; a `fn` signature ends before the body
/// brace, so body-only edits are signature-identical; `const` / `static` /
/// `type` / `use` keep the whole line minus the trailing `;` because the
/// value / aliased type / path is the surface of those kinds; a balanced
/// one-line item keeps the whole line, while a multiline item keeps only
/// the header before the first `{`.
fn declaration_signature(kind: &str, line: &str) -> String {
    let normalized = line.split_whitespace().collect::<Vec<_>>().join(" ");
    let strip_trailing_semicolon = |text: &str| {
        text.trim_end()
            .trim_end_matches(';')
            .trim_end()
            .to_string()
    };
    match kind {
        "const" | "static" | "type" | "use" => strip_trailing_semicolon(&normalized),
        "fn" => {
            let head = match normalized.find('{') {
                Some(idx) => &normalized[..idx],
                None => &normalized[..],
            };
            strip_trailing_semicolon(head)
        }
        "struct" | "enum" | "trait" | "union" | "mod" => {
            let opens = normalized.matches('{').count();
            let closes = normalized.matches('}').count();
            if opens > 0 && opens == closes && normalized.ends_with('}') {
                normalized
            } else if let Some(idx) = normalized.find('{') {
                normalized[..idx].trim_end().to_string()
            } else {
                strip_trailing_semicolon(&normalized)
            }
        }
        _ => normalized,
    }
}

/// Extracts the public surface as a map from declaration key to the set
/// of normalized signatures seen for that key. SS7 identifiers enter as
/// key = signature = identifier, so a changed line carrying one still
/// registers exactly as it does in the name set.
fn extract_public_surface_signatures(
    text: &str,
) -> std::collections::BTreeMap<String, std::collections::BTreeSet<String>> {
    let mut signatures: std::collections::BTreeMap<String, std::collections::BTreeSet<String>> =
        std::collections::BTreeMap::new();
    for ident in extract_public_surface_identifiers(text) {
        signatures.entry(ident.clone()).or_default().insert(ident);
    }
    for line in text.lines() {
        if let Some((name, kind)) = parse_public_surface_declaration(line) {
            let signature = declaration_signature(&kind, line);
            signatures.entry(name).or_default().insert(signature);
        }
    }
    signatures
}

fn watched_changed_lines(root: &Path, rel_path: &str) -> (Vec<String>, Vec<String>) {
    if root.join(".git").exists() {
        let tracked = std::process::Command::new("git")
            .args(["ls-files", "--error-unmatch", "--", rel_path])
            .current_dir(root)
            .output()
            .map(|out| out.status.success())
            .unwrap_or(false);
        let mut added = Vec::new();
        let mut removed = Vec::new();
        let mut inspected = false;
        // Shared committed baseline (connector::committed_diff_range):
        // @{u}..HEAD, else origin/main..HEAD, else no committed range —
        // deliberately no silent HEAD~1..HEAD degradation.
        let range = crate::workflow::connector::committed_diff_range(root);
        let mut diff_paths = vec![rel_path.to_string()];
        if let Some(source) = range
            .as_deref()
            .and_then(|range| rename_source(root, range, rel_path))
            .or_else(|| rename_source(root, "HEAD", rel_path))
        {
            // Both endpoints are required for Git to recognize pure renames.
            diff_paths.insert(0, source);
        }
        let mut diffs: Vec<Vec<String>> = Vec::new();
        if let Some(range) = &range {
            let mut args = vec![
                "diff".into(),
                "-U0".into(),
                "-M".into(),
                range.clone(),
                "--".into(),
            ];
            args.extend(diff_paths.clone());
            diffs.push(args);
        }
        let mut args = vec![
            "diff".into(),
            "-U0".into(),
            "-M".into(),
            "HEAD".into(),
            "--".into(),
        ];
        args.extend(diff_paths);
        diffs.push(args);
        for args in diffs
            .iter()
            .map(|v| v.iter().map(|s| s.as_str()).collect::<Vec<_>>())
        {
            if let Ok(out) = std::process::Command::new("git")
                .args(&args)
                .current_dir(root)
                .output()
            {
                if out.status.success() {
                    inspected = true;
                    for line in String::from_utf8_lossy(&out.stdout).lines() {
                        if line.starts_with('+') && !line.starts_with("+++") {
                            added.push(line[1..].to_string());
                        } else if line.starts_with('-') && !line.starts_with("---") {
                            removed.push(line[1..].to_string());
                        }
                    }
                }
            }
        }
        // Successful Git inspection is authoritative, including deletion-only
        // diffs. Only new/untracked files or failed Git inspection fall back.
        if tracked && inspected {
            return (added, removed);
        }
    }
    let fallback = std::fs::read_to_string(root.join(rel_path)).unwrap_or_default();
    (
        fallback.lines().map(|line| line.to_string()).collect(),
        Vec::new(),
    )
}

fn watched_added_lines(root: &Path, rel_path: &str) -> String {
    watched_changed_lines(root, rel_path).0.join("\n")
}

pub fn evaluate_doc_gate(
    root: &Path,
    changed_paths_override: Option<Vec<String>>,
) -> DocGateReport {
    // The doc gate is a trust-bearing consumer (PR #206 V-3): its changed
    // set must be observed by the Git connector, never declared by a
    // repo-supplied manifest connector.
    use crate::workflow::connector::WorkflowConnector as _;
    let connector = crate::workflow::connector::GitWorkflowConnector;
    let changed_paths = changed_paths_override.unwrap_or_else(|| connector.changed_paths(root));

    let mut violations: Vec<DocViolation> = Vec::new();
    let checked_tiers: usize;
    let checked_rules: usize;

    // 3. Load docs/manifest.json; missing or invalid JSON fails the gate.
    let manifest = match load_manifest(root) {
        Ok(m) => m,
        Err(e) => {
            return DocGateReport {
                passed: false,
                connector_id: connector.connector_id().to_string(),
                checked_tiers: 0,
                checked_rules: 0,
                changed_paths,
                violations: vec![DocViolation {
                    rule_id: "manifest".to_string(),
                    kind: "missing_or_invalid_manifest".to_string(),
                    path: "docs/manifest.json".to_string(),
                    detail: format!("docs/manifest.json missing or invalid: {e}"),
                }],
            };
        }
    };

    // 4. Tier layout hygiene: only tier + support subdirectories and
    //    allowed_root_files may live directly under docs/.
    let docs_root = root.join("docs");
    let mut hygiene_failed = false;
    for entry in std::fs::read_dir(&docs_root).into_iter().flatten() {
        let Ok(entry) = entry else { continue };
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();
        if path.is_dir() {
            let allowed = manifest.tiers.vision.dir.ends_with(&name)
                || manifest.tiers.features_api.dir.ends_with(&name)
                || manifest.tiers.extend.dir.ends_with(&name)
                || manifest.support_dirs.iter().any(|d| d.ends_with(&name));
            if !allowed {
                hygiene_failed = true;
                violations.push(DocViolation {
                    rule_id: "tier-layout".to_string(),
                    kind: "unclassified_docs_directory".to_string(),
                    path: path.to_string_lossy().to_string(),
                    detail: format!("unclassified docs directory '{}' at docs/ root", name),
                });
            }
        } else if path.is_file() {
            if !manifest.allowed_root_files.iter().any(|f| f == &name) {
                hygiene_failed = true;
                violations.push(DocViolation {
                    rule_id: "tier-layout".to_string(),
                    kind: "unclassified_root_file".to_string(),
                    path: path.to_string_lossy().to_string(),
                    detail: format!("unclassified docs file '{}' at docs/ root", name),
                });
            }
        }
    }

    // 5. Prohibited filename substrings (competitor naming) recursively across docs/.
    let mut all_doc_files = Vec::new();
    collect_files_recursive(&docs_root, &mut all_doc_files);
    for path in all_doc_files {
        let Some(fname) = path.file_name() else {
            continue;
        };
        let name = fname.to_string_lossy().to_lowercase();
        if let Some(sub) = manifest
            .prohibited_filename_substrings
            .iter()
            .find(|s| name.contains(&s.to_lowercase()))
        {
            hygiene_failed = true;
            violations.push(DocViolation {
                rule_id: "competitor-naming".to_string(),
                kind: "prohibited_filename".to_string(),
                path: path.to_string_lossy().to_string(),
                detail: format!("prohibited filename substring '{}'", sub),
            });
        }
    }

    // 5b. No maintainer machine paths in any tracked doc under docs/ (including
    // support_dirs such as docs/superpowers/ and docs/releases/).
    const MACHINE_PATH_NEEDLES: [&str; 2] = ["/home/kenton", "/tmp/opencode"];
    for path in collect_doc_markdown_files(root) {
        let Ok(content) = std::fs::read_to_string(&path) else {
            continue;
        };
        let found: Vec<&str> = MACHINE_PATH_NEEDLES
            .iter()
            .copied()
            .filter(|needle| content.contains(needle))
            .collect();
        if found.is_empty() {
            continue;
        }
        hygiene_failed = true;
        let rel = path
            .strip_prefix(root)
            .unwrap_or(&path)
            .to_string_lossy()
            .to_string();
        violations.push(DocViolation {
            rule_id: "machine-path".to_string(),
            kind: "maintainer_machine_path".to_string(),
            path: rel.clone(),
            detail: format!(
                "maintainer machine path(s) {:?} found in tracked doc '{}'",
                found, rel
            ),
        });
    }

    if hygiene_failed {
        return DocGateReport {
            passed: false,
            connector_id: connector.connector_id().to_string(),
            checked_tiers: 0,
            checked_rules: 0,
            changed_paths,
            violations,
        };
    }

    // 6. Check all 3 tiers.
    checked_tiers = 3;
    for (tier_name, tier_spec) in [
        ("vision", &manifest.tiers.vision),
        ("features_api", &manifest.tiers.features_api),
        ("extend", &manifest.tiers.extend),
    ] {
        let tier_dir = root.join(&tier_spec.dir);
        if !tier_dir.exists() {
            violations.push(DocViolation {
                rule_id: tier_name.to_string(),
                kind: "missing_required_tier_file".to_string(),
                path: tier_dir.to_string_lossy().to_string(),
                detail: format!("tier directory '{}' missing", tier_spec.dir),
            });
            continue;
        }
        for required_file in &tier_spec.required_files {
            let file_path = if required_file.starts_with("docs/") {
                root.join(required_file)
            } else {
                tier_dir.join(required_file)
            };
            let len = file_path.metadata().map(|m| m.len()).unwrap_or(0);
            if !file_path.exists() || len == 0 {
                violations.push(DocViolation {
                    rule_id: tier_name.to_string(),
                    kind: "missing_required_tier_file".to_string(),
                    path: file_path.to_string_lossy().to_string(),
                    detail: format!("required file '{}' missing or empty", required_file),
                });
            }
        }
    }

    // 7. Evaluate surface rules.
    checked_rules = manifest.surface_rules.len();
    for rule in &manifest.surface_rules {
        if !["vision", "features_api", "extend"].contains(&rule.tier.as_str()) {
            violations.push(DocViolation {
                rule_id: rule.id.clone(),
                kind: "tier_boundary_mismatch".to_string(),
                path: String::new(),
                detail: format!("invalid tier '{}' in surface rule {}", rule.tier, rule.id),
            });
            continue;
        }
        let expected_prefix = match rule.tier.as_str() {
            "vision" => manifest.tiers.vision.dir.as_str(),
            "features_api" => manifest.tiers.features_api.dir.as_str(),
            _ => manifest.tiers.extend.dir.as_str(),
        };
        if !rule.target_doc.starts_with(expected_prefix) {
            violations.push(DocViolation {
                rule_id: rule.id.clone(),
                kind: "tier_boundary_mismatch".to_string(),
                path: rule.target_doc.clone(),
                detail: format!(
                    "surface rule {} target_doc '{}' outside tier {}",
                    rule.id, rule.target_doc, rule.tier
                ),
            });
            continue;
        }
        let target_doc_path = if rule.target_doc.starts_with("docs/") {
            root.join(&rule.target_doc)
        } else {
            root.join("docs").join(&rule.target_doc)
        };
        let target_len = target_doc_path.metadata().map(|m| m.len()).unwrap_or(0);
        if !target_doc_path.exists() || target_len == 0 {
            violations.push(DocViolation {
                rule_id: rule.id.clone(),
                kind: "missing_target_doc".to_string(),
                path: target_doc_path.to_string_lossy().to_string(),
                detail: format!("target_doc '{}' missing or empty", rule.target_doc),
            });
            continue;
        }
        let content = std::fs::read_to_string(&target_doc_path).unwrap_or_default();
        for heading in &rule.required_headings {
            if !content.contains(heading) {
                violations.push(DocViolation {
                    rule_id: rule.id.clone(),
                    kind: "missing_required_heading".to_string(),
                    path: target_doc_path.to_string_lossy().to_string(),
                    detail: format!(
                        "required heading '{}' missing in {}",
                        heading, rule.target_doc
                    ),
                });
            }
        }
        for token in &rule.required_tokens {
            if !content.contains(token) {
                violations.push(DocViolation {
                    rule_id: rule.id.clone(),
                    kind: "missing_required_token".to_string(),
                    path: target_doc_path.to_string_lossy().to_string(),
                    detail: format!("required token '{}' missing in {}", token, rule.target_doc),
                });
            }
        }
        for cp in &changed_paths {
            let matches_watch = rule
                .watch_paths
                .iter()
                .any(|wp| cp == wp || cp.starts_with(wp));
            if matches_watch {
                let added = watched_added_lines(root, cp);
                for ident in extract_public_surface_identifiers(&added) {
                    if !content.contains(&ident) {
                        violations.push(DocViolation {
                            rule_id: rule.id.clone(),
                            kind: "undocumented_public_identifier".to_string(),
                            path: rule.target_doc.clone(),
                            detail: format!(
                                "added public identifier '{}' in '{}' is not documented in {}",
                                ident, cp, rule.target_doc
                            ),
                        });
                    }
                }
            }
        }
    }

    // 8. Surface-diff check: watched public-surface change without doc update.
    //    Fires only when the watched change implicates the public surface:
    //    the rule's merged added/removed signature maps differ — a public
    //    declaration or SS7 identifier was added, removed, or changed
    //    signature on a diff line of one of the rule's changed watched
    //    paths. A body-only edit to a declaration line yields identical
    //    signatures on both sides and does not fire.
    for rule in &manifest.surface_rules {
        if !rule.require_co_change_on_surface_change {
            continue;
        }
        let watched_changed: Vec<&String> = changed_paths
            .iter()
            .filter(|cp| {
                rule.watch_paths
                    .iter()
                    .any(|wp| *cp == wp || cp.starts_with(wp))
            })
            .collect();
        let doc_changed = changed_paths.iter().any(|cp| cp == &rule.target_doc);
        if watched_changed.is_empty() || doc_changed {
            continue;
        }
        let mut added_signatures: std::collections::BTreeMap<
            String,
            std::collections::BTreeSet<String>,
        > = std::collections::BTreeMap::new();
        let mut removed_signatures: std::collections::BTreeMap<
            String,
            std::collections::BTreeSet<String>,
        > = std::collections::BTreeMap::new();
        for cp in &watched_changed {
            let (added, removed) = watched_changed_lines(root, cp);
            for (key, sigs) in extract_public_surface_signatures(&added.join("\n")) {
                added_signatures.entry(key).or_default().extend(sigs);
            }
            for (key, sigs) in extract_public_surface_signatures(&removed.join("\n")) {
                removed_signatures.entry(key).or_default().extend(sigs);
            }
        }
        if added_signatures != removed_signatures {
            violations.push(DocViolation {
                rule_id: rule.id.clone(),
                kind: "surface_changed_without_doc_update".to_string(),
                path: rule.target_doc.clone(),
                detail: format!(
                    "watched public surface changed without target_doc update: {}",
                    rule.target_doc
                ),
            });
        }
    }

    let passed = violations.is_empty();
    DocGateReport {
        passed,
        connector_id: connector.connector_id().to_string(),
        checked_tiers,
        checked_rules,
        changed_paths,
        violations,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_manifest(docs_dir: &std::path::Path, surface_rules_json: &str) {
        let manifest = format!(
            r#"{{
            "schema_version": 1,
            "allowed_root_files": ["README.md", "manifest.json"],
            "tiers": {{
                "vision": {{
                    "dir": "docs/vision",
                    "description": "Vision tier",
                    "required_files": []
                }},
                "features_api": {{
                    "dir": "docs/features-api",
                    "description": "Features API tier",
                    "required_files": []
                }},
                "extend": {{
                    "dir": "docs/extend",
                    "description": "Extend tier",
                    "required_files": []
                }}
            }},
            "support_dirs": ["docs/releases", "docs/assets"],
            "prohibited_filename_substrings": [],
            "surface_rules": {}
        }}"#,
            surface_rules_json
        );
        std::fs::write(docs_dir.join("manifest.json"), manifest).unwrap();
    }

    fn write_manifest_support_dirs(docs_dir: &std::path::Path, support_dirs_json: &str) {
        let manifest = format!(
            r#"{{
            "schema_version": 1,
            "allowed_root_files": ["README.md", "manifest.json"],
            "tiers": {{
                "vision": {{
                    "dir": "docs/vision",
                    "description": "Vision tier",
                    "required_files": []
                }},
                "features_api": {{
                    "dir": "docs/features-api",
                    "description": "Features API tier",
                    "required_files": []
                }},
                "extend": {{
                    "dir": "docs/extend",
                    "description": "Extend tier",
                    "required_files": []
                }}
            }},
            "support_dirs": {},
            "prohibited_filename_substrings": [],
            "surface_rules": []
        }}"#,
            support_dirs_json
        );
        std::fs::write(docs_dir.join("manifest.json"), manifest).unwrap();
    }

    fn run_git(root: &Path, args: &[&str]) {
        let out = std::process::Command::new("git")
            .arg("-C")
            .arg(root)
            .args(args)
            .output()
            .expect("git available in test env");
        assert!(
            out.status.success(),
            "git {:?} failed: {}",
            args,
            String::from_utf8_lossy(&out.stderr)
        );
    }

    fn init_git_repo(root: &Path) {
        run_git(root, &["init", "-q"]);
        run_git(root, &["config", "user.email", "worker@example.com"]);
        run_git(root, &["config", "user.name", "worker"]);
    }

    #[test]
    fn rejects_unclassified_root_markdown_file() {
        let tmp_dir = tempfile::tempdir().unwrap();
        let docs_dir = tmp_dir.path().join("docs");
        std::fs::create_dir_all(&docs_dir).unwrap();
        std::fs::write(docs_dir.join("unclassified.md"), "# Bad File").unwrap();
        write_manifest(&docs_dir, "[]");
        std::fs::create_dir_all(docs_dir.join("vision")).unwrap();

        let report = evaluate_doc_gate(tmp_dir.path(), None);
        assert!(!report.passed);
        assert_eq!(report.violations.len(), 1);
        assert_eq!(report.violations[0].kind, "unclassified_root_file");
        assert_eq!(report.violations[0].rule_id, "tier-layout");
    }

    #[test]
    fn rejects_prohibited_competitor_filename() {
        let tmp_dir = tempfile::tempdir().unwrap();
        let docs_dir = tmp_dir.path().join("docs");
        std::fs::create_dir_all(&docs_dir.join("vision")).unwrap();
        let bad_token_a = concat!("cog", "nee");
        let bad_token_b = concat!("oxi", "rs");
        let bad_name = format!("design-{}", bad_token_a);
        std::fs::write(
            docs_dir.join("vision").join(format!("{}.md", bad_name)),
            "# Bad",
        )
        .unwrap();
        let manifest = serde_json::json!({
            "schema_version": 1,
            "allowed_root_files": ["README.md", "manifest.json"],
            "tiers": {
                "vision": { "dir": "docs/vision", "description": "Vision tier", "required_files": [] },
                "features_api": { "dir": "docs/features-api", "description": "Features API tier", "required_files": [] },
                "extend": { "dir": "docs/extend", "description": "Extend tier", "required_files": [] }
            },
            "support_dirs": ["docs/releases", "docs/assets"],
            "prohibited_filename_substrings": [bad_token_a, bad_token_b],
            "surface_rules": []
        })
        .to_string();
        std::fs::write(docs_dir.join("manifest.json"), manifest).unwrap();

        let report = evaluate_doc_gate(tmp_dir.path(), None);
        assert!(!report.passed);
        assert_eq!(report.violations.len(), 1);
        assert_eq!(report.violations[0].kind, "prohibited_filename");
        assert_eq!(report.violations[0].rule_id, "competitor-naming");
    }

    #[test]
    fn rejects_missing_required_heading_or_token_in_mapped_doc() {
        let tmp_dir = tempfile::tempdir().unwrap();
        let docs_dir = tmp_dir.path().join("docs");
        std::fs::create_dir_all(docs_dir.join("vision")).unwrap();
        std::fs::create_dir_all(docs_dir.join("features-api")).unwrap();
        std::fs::create_dir_all(docs_dir.join("extend")).unwrap();
        std::fs::write(
            docs_dir.join("vision").join("target_doc.md"),
            "## Introduction\nMissing token here.",
        )
        .unwrap();
        let manifest = r###"{
            "schema_version": 1,
            "allowed_root_files": ["README.md", "manifest.json"],
            "tiers": {
                "vision": { "dir": "docs/vision", "description": "Vision tier", "required_files": [] },
                "features_api": { "dir": "docs/features-api", "description": "Features API tier", "required_files": [] },
                "extend": { "dir": "docs/extend", "description": "Extend tier", "required_files": [] }
            },
            "support_dirs": ["docs/releases", "docs/assets"],
            "prohibited_filename_substrings": [],
            "surface_rules": [
                {
                    "id": "test-rule-1",
                    "tier": "vision",
                    "watch_paths": [],
                    "target_doc": "docs/vision/target_doc.md",
                    "required_headings": ["## Introduction"],
                    "required_tokens": ["required-missing-token"],
                    "require_co_change_on_surface_change": false
                }
            ]
        }"###;
        std::fs::write(docs_dir.join("manifest.json"), manifest).unwrap();

        let report = evaluate_doc_gate(tmp_dir.path(), None);
        assert!(!report.passed);
        assert_eq!(report.violations.len(), 1);
        assert!(
            report.violations[0].kind == "missing_required_heading"
                || report.violations[0].kind == "missing_required_token"
        );
        assert_eq!(report.violations[0].rule_id, "test-rule-1");
    }

    #[test]
    fn internal_change_passes_without_doc_churn() {
        let tmp_dir = tempfile::tempdir().unwrap();
        let docs_dir = tmp_dir.path().join("docs");
        std::fs::create_dir_all(docs_dir.join("vision")).unwrap();
        std::fs::create_dir_all(docs_dir.join("features-api")).unwrap();
        std::fs::create_dir_all(docs_dir.join("extend")).unwrap();
        std::fs::write(
            docs_dir.join("vision").join("target_doc.md"),
            "## Introduction\nThe core target_doc and feature one.",
        )
        .unwrap();
        let manifest = r###"{
            "schema_version": 1,
            "allowed_root_files": ["README.md", "manifest.json"],
            "tiers": {
                "vision": { "dir": "docs/vision", "description": "Vision tier", "required_files": [] },
                "features_api": { "dir": "docs/features-api", "description": "Features API tier", "required_files": [] },
                "extend": { "dir": "docs/extend", "description": "Extend tier", "required_files": [] }
            },
            "support_dirs": ["docs/releases", "docs/assets"],
            "prohibited_filename_substrings": [],
            "surface_rules": [
                {
                    "id": "test-rule-1",
                    "tier": "vision",
                    "watch_paths": ["src/internal/module.rs"],
                    "target_doc": "docs/vision/target_doc.md",
                    "required_headings": ["## Introduction"],
                    "required_tokens": ["target_doc", "feature one"],
                    "require_co_change_on_surface_change": false
                }
            ]
        }"###;
        std::fs::write(docs_dir.join("manifest.json"), manifest).unwrap();

        let report = evaluate_doc_gate(
            tmp_dir.path(),
            Some(vec!["src/internal/module.rs".to_string()]),
        );
        assert!(report.passed);
        assert_eq!(report.violations.len(), 0);
    }

    #[test]
    fn surface_change_missing_co_change_or_token_blocks() {
        let tmp_dir = tempfile::tempdir().unwrap();
        let docs_dir = tmp_dir.path().join("docs");
        std::fs::create_dir_all(docs_dir.join("vision")).unwrap();
        std::fs::create_dir_all(docs_dir.join("features-api")).unwrap();
        std::fs::create_dir_all(docs_dir.join("extend")).unwrap();
        std::fs::create_dir_all(tmp_dir.path().join("src")).unwrap();
        std::fs::write(
            tmp_dir.path().join("src").join("server.rs"),
            "pub struct NewSurfaceParams {}\n",
        )
        .unwrap();
        std::fs::write(
            docs_dir.join("vision").join("target_doc.md"),
            "## Introduction\nThe target_doc surface.",
        )
        .unwrap();
        let manifest = r###"{
            "schema_version": 1,
            "allowed_root_files": ["README.md", "manifest.json"],
            "tiers": {
                "vision": { "dir": "docs/vision", "description": "Vision tier", "required_files": [] },
                "features_api": { "dir": "docs/features-api", "description": "Features API tier", "required_files": [] },
                "extend": { "dir": "docs/extend", "description": "Extend tier", "required_files": [] }
            },
            "support_dirs": ["docs/releases", "docs/assets"],
            "prohibited_filename_substrings": [],
            "surface_rules": [
                {
                    "id": "test-rule-1",
                    "tier": "vision",
                    "watch_paths": ["src/server.rs"],
                    "target_doc": "docs/vision/target_doc.md",
                    "required_headings": ["## Introduction"],
                    "required_tokens": ["target_doc"],
                    "require_co_change_on_surface_change": true
                }
            ]
        }"###;
        std::fs::write(docs_dir.join("manifest.json"), manifest).unwrap();

        let report = evaluate_doc_gate(tmp_dir.path(), Some(vec!["src/server.rs".to_string()]));
        assert!(!report.passed);
        assert_eq!(report.violations.len(), 1);
        assert_eq!(
            report.violations[0].kind,
            "surface_changed_without_doc_update"
        );
    }

    #[test]
    fn rejects_undocumented_added_public_identifier_in_watched_source() {
        let tmp_dir = tempfile::tempdir().unwrap();
        let docs_dir = tmp_dir.path().join("docs");
        let src_dir = tmp_dir.path().join("src");
        std::fs::create_dir_all(docs_dir.join("vision")).unwrap();
        std::fs::create_dir_all(docs_dir.join("features-api")).unwrap();
        std::fs::create_dir_all(docs_dir.join("extend")).unwrap();
        std::fs::create_dir_all(&src_dir).unwrap();
        std::fs::write(
            docs_dir.join("features-api").join("cli-reference.md"),
            "## Workflow, Documentation Gate & Ontology Commands\ndoc-gate workflow",
        )
        .unwrap();
        std::fs::write(
            src_dir.join("main.rs"),
            "#[command(name = \"undocumented-new-cmd\")]\nstruct Foo;",
        )
        .unwrap();
        let manifest = r###"{
            "schema_version": 1,
            "allowed_root_files": ["README.md", "manifest.json"],
            "tiers": {
                "vision": { "dir": "docs/vision", "description": "Vision tier", "required_files": [] },
                "features_api": { "dir": "docs/features-api", "description": "Features API tier", "required_files": [] },
                "extend": { "dir": "docs/extend", "description": "Extend tier", "required_files": [] }
            },
            "support_dirs": ["docs/releases", "docs/assets"],
            "prohibited_filename_substrings": [],
            "surface_rules": [
                {
                    "id": "cli-reference-surface",
                    "tier": "features_api",
                    "watch_paths": ["src/main.rs"],
                    "target_doc": "docs/features-api/cli-reference.md",
                    "required_headings": ["## Workflow, Documentation Gate & Ontology Commands"],
                    "required_tokens": ["doc-gate", "workflow"],
                    "require_co_change_on_surface_change": false
                }
            ]
        }"###;
        std::fs::write(docs_dir.join("manifest.json"), manifest).unwrap();

        let report = evaluate_doc_gate(tmp_dir.path(), Some(vec!["src/main.rs".to_string()]));
        assert!(!report.passed);
        assert!(report.violations.iter().any(|v| {
            v.kind == "undocumented_public_identifier" && v.detail.contains("undocumented-new-cmd")
        }));
    }

    #[test]
    fn committed_surface_change_without_doc_update_blocks_on_production_path() {
        // No changed_paths_override: the gate must see a real committed
        // changed set (baseline pinned at refs/remotes/origin/main).
        let tmp_dir = tempfile::tempdir().unwrap();
        let root = tmp_dir.path();
        let docs_dir = root.join("docs");
        std::fs::create_dir_all(docs_dir.join("vision")).unwrap();
        std::fs::create_dir_all(docs_dir.join("features-api")).unwrap();
        std::fs::create_dir_all(docs_dir.join("extend")).unwrap();
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(
            docs_dir.join("vision").join("target_doc.md"),
            "## Introduction\nThe target_doc surface.",
        )
        .unwrap();
        std::fs::write(
            root.join("src").join("server.rs"),
            "pub fn surface_v1() {}\n",
        )
        .unwrap();
        let manifest = r###"{
            "schema_version": 1,
            "allowed_root_files": ["README.md", "manifest.json"],
            "tiers": {
                "vision": { "dir": "docs/vision", "description": "Vision tier", "required_files": [] },
                "features_api": { "dir": "docs/features-api", "description": "Features API tier", "required_files": [] },
                "extend": { "dir": "docs/extend", "description": "Extend tier", "required_files": [] }
            },
            "support_dirs": ["docs/releases", "docs/assets"],
            "prohibited_filename_substrings": [],
            "surface_rules": [
                {
                    "id": "test-rule-1",
                    "tier": "vision",
                    "watch_paths": ["src/server.rs"],
                    "target_doc": "docs/vision/target_doc.md",
                    "required_headings": ["## Introduction"],
                    "required_tokens": ["target_doc"],
                    "require_co_change_on_surface_change": true
                }
            ]
        }"###;
        std::fs::write(docs_dir.join("manifest.json"), manifest).unwrap();
        let run = |args: &[&str]| {
            let out = std::process::Command::new("git")
                .arg("-C")
                .arg(root)
                .args(args)
                .output()
                .expect("git available in test env");
            assert!(
                out.status.success(),
                "git {:?} failed: {}",
                args,
                String::from_utf8_lossy(&out.stderr)
            );
        };
        run(&["init", "-q"]);
        run(&["config", "user.email", "worker@example.com"]);
        run(&["config", "user.name", "worker"]);
        run(&["add", "."]);
        run(&["commit", "-q", "-m", "baseline"]);
        run(&["update-ref", "refs/remotes/origin/main", "HEAD"]);
        std::fs::write(
            root.join("src").join("server.rs"),
            "pub struct SurfaceV2 {}\npub fn surface_v2() {}\n",
        )
        .unwrap();
        run(&["add", "."]);
        run(&["commit", "-q", "-m", "surface change without doc update"]);

        let report = evaluate_doc_gate(root, None);
        assert!(
            report.changed_paths.iter().any(|p| p == "src/server.rs"),
            "production changed set must include the committed change, got {:?}",
            report.changed_paths
        );
        assert!(!report.passed);
        assert!(
            report.violations.iter().any(|v| {
                v.kind == "surface_changed_without_doc_update" && v.rule_id == "test-rule-1"
            }),
            "expected co-change violation, got {:?}",
            report.violations
        );
    }

    #[test]
    fn extract_public_surface_declarations_covers_item_kinds_and_qualifiers() {
        let text = "\
pub struct PlainStruct {}
pub(crate) enum ScopedEnum { A }
pub async fn fetch_thing() {}
pub const MAX_DEPTH: usize = 3;
pub static TABLE: &[&str] = &[];
pub type Alias = u64;
pub mod inner {}
pub use crate::store::Widget;
pub use crate::store::Widget as RenamedWidget;
pub unsafe fn raw_op() {}
pub const fn const_op() {}
fn private_fn() {}
struct PrivateStruct {}
    pub trait DocumentedTrait {}
";
        let decls = extract_public_surface_declarations(text);
        for expected in [
            "PlainStruct",
            "ScopedEnum",
            "fetch_thing",
            "MAX_DEPTH",
            "TABLE",
            "Alias",
            "inner",
            "Widget",
            "RenamedWidget",
            "raw_op",
            "const_op",
            "DocumentedTrait",
        ] {
            assert!(decls.contains(expected), "missing {expected}: {decls:?}");
        }
        assert!(!decls.contains("private_fn"), "{decls:?}");
        assert!(!decls.contains("PrivateStruct"), "{decls:?}");
    }

    #[test]
    fn extract_public_surface_signatures_body_only_fn_edit_is_identical() {
        let before = extract_public_surface_signatures("pub fn compute() -> i32 { 1 }");
        let after = extract_public_surface_signatures("pub fn compute() -> i32 { 2 }");
        assert!(before.contains_key("compute"), "{before:?}");
        assert!(after.contains_key("compute"), "{after:?}");
        assert_eq!(before, after);
    }

    #[test]
    fn extract_public_surface_signatures_signature_change_differs() {
        let before = extract_public_surface_signatures("pub fn compute(x: u64) -> i32 { 1 }");
        let after = extract_public_surface_signatures("pub fn compute(x: String) -> i32 { 1 }");
        assert_ne!(before.get("compute"), after.get("compute"));
    }

    #[test]
    fn extract_public_surface_signatures_const_value_change_differs() {
        let before = extract_public_surface_signatures("pub const MAX_DEPTH: usize = 3;");
        let after = extract_public_surface_signatures("pub const MAX_DEPTH: usize = 4;");
        assert_ne!(before.get("MAX_DEPTH"), after.get("MAX_DEPTH"));
    }

    #[test]
    fn extract_public_surface_signatures_whitespace_reformat_is_identical() {
        let before = extract_public_surface_signatures("pub fn compute() -> i32 { 1 }");
        let after = extract_public_surface_signatures("pub  fn   compute()   ->   i32   { 1 }");
        assert_eq!(before, after);
    }

    #[test]
    fn extract_public_surface_signatures_one_line_enum_variant_change_differs() {
        let before = extract_public_surface_signatures("pub enum Thing { A, B }");
        let after = extract_public_surface_signatures("pub enum Thing { A, C }");
        assert_ne!(before.get("Thing"), after.get("Thing"));
    }

    fn setup_git_surface_repo(root: &Path, initial_source: &str) {
        let docs_dir = root.join("docs");
        std::fs::create_dir_all(docs_dir.join("vision")).unwrap();
        std::fs::create_dir_all(docs_dir.join("features-api")).unwrap();
        std::fs::create_dir_all(docs_dir.join("extend")).unwrap();
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(
            docs_dir.join("vision").join("target_doc.md"),
            "## Introduction\nThe target_doc surface. TargetTrait\n",
        )
        .unwrap();
        std::fs::write(root.join("src").join("server.rs"), initial_source).unwrap();
        write_manifest(
            &docs_dir,
            r###"[{
                "id": "test-rule-1",
                "tier": "vision",
                "watch_paths": ["src/server.rs"],
                "target_doc": "docs/vision/target_doc.md",
                "required_headings": ["## Introduction"],
                "required_tokens": ["target_doc"],
                "require_co_change_on_surface_change": true
            }]"###,
        );
        init_git_repo(root);
        run_git(root, &["add", "."]);
        run_git(root, &["commit", "-q", "-m", "baseline"]);
        run_git(root, &["update-ref", "refs/remotes/origin/main", "HEAD"]);
    }

    fn commit_all(root: &Path, message: &str) {
        run_git(root, &["add", "."]);
        run_git(root, &["commit", "-q", "-m", message]);
    }

    #[test]
    fn internal_change_to_watched_file_passes_without_doc_update() {
        let tmp_dir = tempfile::tempdir().unwrap();
        let root = tmp_dir.path();
        setup_git_surface_repo(root, "pub trait TargetTrait {}\nfn helper() -> i32 { 1 }\n");
        std::fs::write(
            root.join("src").join("server.rs"),
            "pub trait TargetTrait {}\nfn helper() -> i32 { 2 }\n",
        )
        .unwrap();
        commit_all(root, "internal change");

        let report = evaluate_doc_gate(root, None);
        assert!(
            report.passed,
            "internal-only change produced violations: {:?}",
            report.violations
        );
    }

    #[test]
    fn test_only_change_in_watched_file_passes_without_doc_update() {
        let tmp_dir = tempfile::tempdir().unwrap();
        let root = tmp_dir.path();
        setup_git_surface_repo(
            root,
            "pub trait TargetTrait {}\nfn check(value: i32) -> i32 { value }\n#[cfg(test)]\nmod tests {\n    use super::check;\n    #[test]\n    fn it_works() { assert_eq!(check(1), 1); }\n}\n",
        );
        std::fs::write(
            root.join("src").join("server.rs"),
            "pub trait TargetTrait {}\nfn check(value: i32) -> i32 { value }\n#[cfg(test)]\nmod tests {\n    use super::check;\n    #[test]\n    fn it_works() { assert_eq!(check(2), 2); }\n}\n",
        )
        .unwrap();
        commit_all(root, "test-only change");

        let report = evaluate_doc_gate(root, None);
        assert!(
            report.passed,
            "test-only change produced violations: {:?}",
            report.violations
        );
    }

    #[test]
    fn attribute_only_change_in_watched_file_passes_without_doc_update() {
        let tmp_dir = tempfile::tempdir().unwrap();
        let root = tmp_dir.path();
        setup_git_surface_repo(root, "pub trait TargetTrait {}\npub enum Thing { A, B }\n");
        std::fs::write(
            root.join("src").join("server.rs"),
            "pub trait TargetTrait {}\n#[allow(dead_code)]\npub enum Thing { A, B }\n",
        )
        .unwrap();
        commit_all(root, "attribute-only change");

        let report = evaluate_doc_gate(root, None);
        assert!(
            report.passed,
            "attribute-only change produced violations: {:?}",
            report.violations
        );
    }

    #[test]
    fn genuine_public_surface_change_without_doc_update_blocks_on_production_path() {
        // T12-style: a new pub struct params type plus a pub trait method
        // signature re-pointed at it, with no doc update.
        let tmp_dir = tempfile::tempdir().unwrap();
        let root = tmp_dir.path();
        setup_git_surface_repo(
            root,
            "pub trait TargetTrait {\n    fn fetch(&self, id: u64);\n}\n",
        );
        std::fs::write(
            root.join("src").join("server.rs"),
            "pub struct FetchParams { pub id: u64 }\npub trait TargetTrait {\n    fn fetch(&self, params: FetchParams);\n}\n",
        )
        .unwrap();
        commit_all(root, "genuine surface change");

        let report = evaluate_doc_gate(root, None);
        assert!(!report.passed);
        assert!(
            report.violations.iter().any(|v| {
                v.kind == "surface_changed_without_doc_update" && v.rule_id == "test-rule-1"
            }),
            "expected co-change violation, got {:?}",
            report.violations
        );
    }

    #[test]
    fn body_only_change_to_one_line_pub_fn_passes_without_doc_update() {
        let tmp_dir = tempfile::tempdir().unwrap();
        let root = tmp_dir.path();
        setup_git_surface_repo(
            root,
            "pub trait TargetTrait {}\npub fn compute() -> i32 { 1 }\n",
        );
        std::fs::write(
            root.join("src").join("server.rs"),
            "pub trait TargetTrait {}\npub fn compute() -> i32 { 2 }\n",
        )
        .unwrap();
        commit_all(root, "body-only change");

        let report = evaluate_doc_gate(root, None);
        assert!(
            report.passed,
            "body-only change produced violations: {:?}",
            report.violations
        );
        assert_eq!(report.violations.len(), 0);
    }

    #[test]
    fn signature_change_to_one_line_pub_fn_blocks_without_doc_update() {
        let tmp_dir = tempfile::tempdir().unwrap();
        let root = tmp_dir.path();
        setup_git_surface_repo(
            root,
            "pub trait TargetTrait {}\npub fn compute() -> i32 { 1 }\n",
        );
        std::fs::write(
            root.join("src").join("server.rs"),
            "pub trait TargetTrait {}\npub fn compute(x: u64) -> i32 { 1 }\n",
        )
        .unwrap();
        commit_all(root, "signature change");

        let report = evaluate_doc_gate(root, None);
        assert!(!report.passed);
        assert!(
            report.violations.iter().any(|v| {
                v.kind == "surface_changed_without_doc_update" && v.rule_id == "test-rule-1"
            }),
            "expected co-change violation, got {:?}",
            report.violations
        );
    }

    #[test]
    fn added_tool_identifier_in_watched_file_still_blocks_via_ss7() {
        // Mutation / non-vacuity: the SS7 identifier check must still fire
        // under the narrowed co-change watch.
        let tmp_dir = tempfile::tempdir().unwrap();
        let root = tmp_dir.path();
        setup_git_surface_repo(root, "pub trait TargetTrait {}\n");
        std::fs::write(
            root.join("src").join("server.rs"),
            "pub trait TargetTrait {}\n// \"name\": \"neurostrata_brand_new_tool\"\n",
        )
        .unwrap();
        commit_all(root, "add tool identifier");

        let report = evaluate_doc_gate(root, None);
        assert!(
            report.violations.iter().any(|v| {
                v.kind == "undocumented_public_identifier"
                    && v.detail.contains("neurostrata_brand_new_tool")
            }),
            "expected SS7 undocumented_public_identifier violation, got {:?}",
            report.violations
        );
    }

    #[test]
    fn deletion_only_watched_change_does_not_scan_remaining_source() {
        let tmp_dir = tempfile::tempdir().unwrap();
        let root = tmp_dir.path();
        let docs_dir = root.join("docs");
        std::fs::create_dir_all(docs_dir.join("vision")).unwrap();
        std::fs::create_dir_all(docs_dir.join("features-api")).unwrap();
        std::fs::create_dir_all(docs_dir.join("extend")).unwrap();
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(
            docs_dir.join("features-api").join("target.md"),
            "# Target\nNo public identifiers documented here.",
        )
        .unwrap();
        std::fs::write(
            root.join("src").join("main.rs"),
            "pub trait LegacyTrait {}\nfn remove_me() {}\n",
        )
        .unwrap();
        write_manifest(
            &docs_dir,
            r#"[{
                "id": "cli-surface",
                "tier": "features_api",
                "watch_paths": ["src/main.rs"],
                "target_doc": "docs/features-api/target.md",
                "required_headings": [],
                "required_tokens": [],
                "require_co_change_on_surface_change": false
            }]"#,
        );
        init_git_repo(root);
        run_git(root, &["add", "."]);
        run_git(root, &["commit", "-q", "-m", "baseline"]);
        run_git(root, &["update-ref", "refs/remotes/origin/main", "HEAD"]);
        std::fs::write(
            root.join("src").join("main.rs"),
            "pub trait LegacyTrait {}\n",
        )
        .unwrap();
        run_git(root, &["add", "src/main.rs"]);
        run_git(root, &["commit", "-q", "-m", "deletion only"]);

        let report = evaluate_doc_gate(root, None);
        assert!(
            report.passed,
            "deletion-only diff produced violations: {:?}",
            report.violations
        );
    }

    #[test]
    fn pure_rename_does_not_treat_existing_source_as_added() {
        let tmp_dir = tempfile::tempdir().unwrap();
        let root = tmp_dir.path();
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(
            root.join("src").join("old.rs"),
            "pub trait LegacyTrait {}\n",
        )
        .unwrap();
        init_git_repo(root);
        run_git(root, &["add", "."]);
        run_git(root, &["commit", "-q", "-m", "baseline"]);
        run_git(root, &["update-ref", "refs/remotes/origin/main", "HEAD"]);
        run_git(root, &["mv", "src/old.rs", "src/new.rs"]);
        run_git(root, &["commit", "-q", "-m", "pure rename"]);

        let added = watched_added_lines(root, "src/new.rs");
        assert!(
            added.is_empty(),
            "pure rename treated existing source as added: {added:?}"
        );
    }

    #[test]
    fn untracked_watched_file_still_scans_new_public_identifiers() {
        let tmp_dir = tempfile::tempdir().unwrap();
        let root = tmp_dir.path();
        let docs_dir = root.join("docs");
        std::fs::create_dir_all(docs_dir.join("vision")).unwrap();
        std::fs::create_dir_all(docs_dir.join("features-api")).unwrap();
        std::fs::create_dir_all(docs_dir.join("extend")).unwrap();
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(
            docs_dir.join("features-api").join("target.md"),
            "# Target\nNo public identifiers documented here.",
        )
        .unwrap();
        write_manifest(
            &docs_dir,
            r#"[{
                "id": "cli-surface",
                "tier": "features_api",
                "watch_paths": ["src/main.rs"],
                "target_doc": "docs/features-api/target.md",
                "required_headings": [],
                "required_tokens": [],
                "require_co_change_on_surface_change": false
            }]"#,
        );
        std::fs::write(root.join("src").join("keep.rs"), "fn keep() {}\n").unwrap();
        init_git_repo(root);
        run_git(root, &["add", "."]);
        run_git(root, &["commit", "-q", "-m", "baseline"]);
        run_git(root, &["update-ref", "refs/remotes/origin/main", "HEAD"]);
        std::fs::write(
            root.join("src").join("main.rs"),
            "pub trait UndocumentedNewTrait {}\n",
        )
        .unwrap();

        let report = evaluate_doc_gate(root, None);
        assert!(
            report.violations.iter().any(|v| {
                v.kind == "undocumented_public_identifier"
                    && v.detail.contains("UndocumentedNewTrait")
            }),
            "untracked file was not scanned: {:?}",
            report
        );
    }

    #[test]
    fn rejects_maintainer_machine_path_in_support_dir_doc() {
        // No .git in a temp dir: every markdown file under docs/ (including
        // support_dirs) must be scanned for maintainer machine paths.
        let tmp_dir = tempfile::tempdir().unwrap();
        let root = tmp_dir.path();
        let docs_dir = root.join("docs");
        std::fs::create_dir_all(docs_dir.join("vision")).unwrap();
        std::fs::create_dir_all(docs_dir.join("features-api")).unwrap();
        std::fs::create_dir_all(docs_dir.join("extend")).unwrap();
        std::fs::create_dir_all(docs_dir.join("superpowers").join("plans")).unwrap();
        write_manifest_support_dirs(&docs_dir, r#"["docs/superpowers"]"#);
        std::fs::write(
            docs_dir.join("superpowers").join("plans").join("leak.md"),
            "See /tmp/opencode/semantic-eval.json for the strict receipt.",
        )
        .unwrap();

        let report = evaluate_doc_gate(root, None);
        assert!(!report.passed);
        assert!(
            report
                .violations
                .iter()
                .any(|v| { v.kind == "maintainer_machine_path" && v.path.contains("leak.md") }),
            "expected maintainer_machine_path violation, got {:?}",
            report.violations
        );
    }

    #[test]
    fn machine_path_rule_ignores_untracked_but_flags_tracked_doc() {
        // Inside a git worktree only tracked docs are scanned, so an
        // untracked scratch file under docs/superpowers/ never blocks the gate.
        let tmp_dir = tempfile::tempdir().unwrap();
        let root = tmp_dir.path();
        let docs_dir = root.join("docs");
        std::fs::create_dir_all(docs_dir.join("vision")).unwrap();
        std::fs::create_dir_all(docs_dir.join("features-api")).unwrap();
        std::fs::create_dir_all(docs_dir.join("extend")).unwrap();
        std::fs::create_dir_all(docs_dir.join("superpowers").join("plans")).unwrap();
        write_manifest_support_dirs(&docs_dir, r#"["docs/superpowers"]"#);
        init_git_repo(root);
        run_git(root, &["add", "."]);
        run_git(root, &["commit", "-q", "-m", "baseline"]);

        let scratch = docs_dir
            .join("superpowers")
            .join("plans")
            .join("scratch.md");
        std::fs::write(&scratch, "run from /home/kenton/worktree/<id>").unwrap();
        let report = evaluate_doc_gate(root, None);
        assert!(
            report.passed,
            "untracked scratch doc must be ignored: {:?}",
            report.violations
        );

        run_git(root, &["add", "docs/superpowers/plans/scratch.md"]);
        let report = evaluate_doc_gate(root, None);
        assert!(
            report
                .violations
                .iter()
                .any(|v| v.kind == "maintainer_machine_path"),
            "tracked doc with machine path must be flagged: {:?}",
            report.violations
        );
    }

    #[test]
    fn doc_gate_passes_on_live_repository() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let report = evaluate_doc_gate(root, None);
        assert!(
            report.passed,
            "live repository failed doc-gate: {:?}",
            report.violations
        );
        assert_eq!(report.checked_tiers, 3);
        assert!(report.checked_rules >= 8);
    }
}
