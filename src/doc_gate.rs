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
    // The legacy spelling `require_co_change_on_touch` is accepted as an
    // alias so a manifest written for the pre-rename field name still
    // enables the rule instead of silently defaulting it off.
    #[serde(default, alias = "require_co_change_on_touch")]
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
/// (declared name, item kind) when the line begins a bare `pub`
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
        // Only bare `pub` is external surface: a restricted visibility
        // group (`pub(crate)`, `pub(super)`, `pub(in ...)`) marks an
        // internal item, which must not trigger public-API checks.
        let rest = if rest.starts_with(char::is_whitespace) {
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
                "extern" => {
                    start += 1;
                    // An ABI string (`"C"`, `"system"`) is a single
                    // whitespace-delimited token; consume it too.
                    if tokens
                        .get(start)
                        .map(|token| token.starts_with('"'))
                        .unwrap_or(false)
                    {
                        start += 1;
                    }
                }
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

/// Joins a use-tree prefix and a path segment with `::`, skipping
/// whichever side is empty.
fn join_use_prefix(prefix: &str, segment: &str) -> String {
    let prefix = prefix.trim();
    let segment = segment.trim();
    if prefix.is_empty() {
        segment.to_string()
    } else if segment.is_empty() {
        prefix.to_string()
    } else {
        format!("{prefix}::{segment}")
    }
}

/// Splits a use-group member list on commas at brace depth 0.
fn split_use_group_members(members: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut depth = 0i32;
    let mut start = 0;
    for (i, c) in members.char_indices() {
        match c {
            '{' => depth += 1,
            '}' => depth -= 1,
            ',' if depth == 0 => {
                parts.push(&members[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    parts.push(&members[start..]);
    parts
}

/// Recursive use-tree parser: expands `path` under `prefix` into
/// (declared name, full path) pairs. A `<head>::{ <members> }` group
/// recurses per member; a leaf honours `as` aliases, maps a glob to
/// the module segment before `*`, maps `self` to the segment before
/// it, and otherwise takes the final segment's identifier run. Empty
/// members and empty identifier runs insert nothing.
fn parse_use_tree(path: &str, prefix: &str, out: &mut Vec<(String, String)>) {
    let path = path.trim();
    if path.is_empty() {
        return;
    }
    if let Some(open) = path.find('{') {
        let head = path[..open].trim().trim_end_matches("::").trim();
        let mut depth = 0i32;
        let mut close = None;
        for (i, c) in path.char_indices().skip_while(|(i, _)| *i < open) {
            match c {
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        close = Some(i);
                        break;
                    }
                }
                _ => {}
            }
        }
        // An unterminated group (a multiline header line) parses its
        // remaining text as members, which yields nothing for the
        // header alone and must not error.
        let members = match close {
            Some(close) => &path[open + 1..close],
            None => &path[open + 1..],
        };
        let joined_prefix = join_use_prefix(prefix, head);
        for member in split_use_group_members(members) {
            let member = member.trim();
            if !member.is_empty() {
                parse_use_tree(member, &joined_prefix, out);
            }
        }
        return;
    }
    let joined = join_use_prefix(prefix, path);
    if let Some((_, alias)) = path.rsplit_once(" as ") {
        let name: String = alias
            .trim()
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
            .collect();
        if !name.is_empty() {
            out.push((name, joined));
        }
        return;
    }
    let final_segment = path.rsplit("::").next().unwrap_or_default().trim();
    if final_segment == "*" {
        let parent = joined
            .strip_suffix("::*")
            .or_else(|| joined.strip_suffix('*'))
            .unwrap_or_default();
        let name: String = parent
            .rsplit("::")
            .next()
            .unwrap_or_default()
            .trim()
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
            .collect();
        if !parent.is_empty() && !name.is_empty() {
            out.push((name, joined));
        }
        return;
    }
    if final_segment == "self" {
        let parent = joined
            .strip_suffix("::self")
            .or_else(|| joined.strip_suffix("self"))
            .unwrap_or_default();
        let name: String = parent
            .rsplit("::")
            .next()
            .unwrap_or_default()
            .trim()
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
            .collect();
        if !parent.is_empty() && !name.is_empty() {
            out.push((name, parent.to_string()));
        }
        return;
    }
    let name: String = final_segment
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
        .collect();
    if !name.is_empty() {
        out.push((name, joined));
    }
}

/// Parses one line as a bare-`pub` `use` declaration,
/// returning its (declared name, full path) pairs via the use-tree
/// parser, or `None` when the line is not a `use` declaration. A
/// `use` line whose tree yields no pairs (e.g. a multiline group
/// header) returns an empty vec.
fn parse_use_declarations(line: &str) -> Option<Vec<(String, String)>> {
    let trimmed = line.trim();
    let rest = trimmed.strip_prefix("pub")?;
    // Bare `pub` only, as in `parse_public_surface_declaration`: a
    // restricted `pub(...)` re-export is internal, not surface.
    let rest = if rest.starts_with(char::is_whitespace) {
        rest
    } else {
        return None;
    };
    let mut tokens: Vec<&str> = rest.split_whitespace().collect();
    let mut start = 0;
    while start < tokens.len() {
        match tokens[start] {
            "async" | "unsafe" | "default" => start += 1,
            "const" if tokens.get(start + 1) == Some(&"fn") => start += 1,
            "extern" => {
                start += 1;
                if tokens
                    .get(start)
                    .map(|token| token.starts_with('"'))
                    .unwrap_or(false)
                {
                    start += 1;
                }
            }
            _ => break,
        }
    }
    tokens.drain(..start);
    if tokens.first() != Some(&"use") {
        return None;
    }
    let path = tokens[1..].join(" ");
    let path = path.trim().trim_end_matches(';').trim();
    let path = path.trim_end_matches(',').trim();
    let mut out = Vec::new();
    parse_use_tree(path, "", &mut out);
    Some(out)
}

/// Parses one line as a restricted-visibility item declaration
/// (`pub(crate)` / `pub(super)` / `pub(in ...)`), returning its
/// (declared name, item kind), or `None` when the line is not one.
/// The line is rewritten to its bare-`pub` form and reparsed by
/// `parse_public_surface_declaration`, so kind / qualifier / name
/// handling stays in one place. `use` lines return `None`: a
/// restricted re-export is internal, never surface.
fn parse_restricted_surface_declaration(line: &str) -> Option<(String, String)> {
    let trimmed = line.trim();
    let rest = trimmed.strip_prefix("pub(")?;
    let close = rest.find(')')?;
    let remainder = rest[close + 1..].trim_start();
    if remainder.is_empty() {
        return None;
    }
    let (name, kind) = parse_public_surface_declaration(&format!("pub {remainder}"))?;
    if kind == "use" {
        return None;
    }
    Some((name, kind))
}

/// Collects the source names re-exported by bare `pub use` lines in
/// `text`: for each (declared name, full path) pair the use-tree
/// parser yields, the item's own name is the final path segment,
/// taken from before any `as` rename (a rename surfaces the alias,
/// but the definition it points at is the restricted item). Glob
/// re-exports yield no source name — the items behind a glob cannot
/// be named from the use line.
fn reexported_source_names(text: &str) -> std::collections::BTreeSet<String> {
    let mut names = std::collections::BTreeSet::new();
    for line in text.lines() {
        if let Some(pairs) = parse_use_declarations(line) {
            for (_declared, full_path) in pairs {
                let source_path = full_path.split(" as ").next().unwrap_or(&full_path);
                let name: String = source_path
                    .rsplit("::")
                    .next()
                    .unwrap_or_default()
                    .trim()
                    .chars()
                    .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
                    .collect();
                if !name.is_empty() {
                    names.insert(name);
                }
            }
        }
    }
    names
}

/// The bare Rust keywords (strict + reserved, 2021 edition, plus `gen`
/// reserved in 2024) that can never name a variant or member in bare
/// spelling; raw spellings (`r#false`) are distinct identifiers.
const RUST_KEYWORDS: [&str; 52] = [
    "as", "async", "await", "break", "const", "continue", "crate", "dyn", "else", "enum",
    "extern", "false", "fn", "for", "if", "impl", "in", "let", "loop", "match", "mod", "move",
    "mut", "pub", "ref", "return", "self", "Self", "static", "struct", "super", "trait", "true",
    "try", "type", "unsafe", "use", "where", "while", "abstract", "become", "box", "do",
    "final", "macro", "override", "priv", "typeof", "unsized", "virtual", "yield", "gen",
];

const PRIMITIVE_TYPES: [&str; 18] = [
    "bool", "char", "str", "i8", "i16", "i32", "i64", "i128", "isize", "u8", "u16", "u32",
    "u64", "u128", "usize", "f32", "f64", "Self",
];

/// Parses an ASCII identifier at the start of `text`, honouring a raw
/// `r#` prefix, returning the identifier as written (prefix included)
/// and its byte length.
fn leading_identifier(text: &str) -> Option<(String, usize)> {
    let (prefix, rest) = if let Some(raw) = text.strip_prefix("r#") {
        ("r#", raw)
    } else {
        ("", text)
    };
    let mut chars = rest.chars();
    let first = chars.next()?;
    if !(first.is_ascii_alphabetic() || first == '_') {
        return None;
    }
    let mut len = first.len_utf8();
    for c in chars {
        if c.is_ascii_alphanumeric() || c == '_' {
            len += c.len_utf8();
        } else {
            break;
        }
    }
    Some((format!("{prefix}{}", &rest[..len]), prefix.len() + len))
}

/// Strips a leading bare-`pub` visibility prefix, returning the
/// trimmed remainder, or `None` when the line does not start with one.
fn strip_visibility_prefix(text: &str) -> Option<&str> {
    let rest = text.strip_prefix("pub")?;
    // Bare `pub` only: `pub(crate)` / `pub(super)` members are internal
    // and do not enter the public surface.
    if rest.starts_with(char::is_whitespace) {
        Some(rest.trim_start())
    } else {
        None
    }
}

fn normalized_line(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn strip_trailing_comma(text: &str) -> &str {
    text.trim_end().strip_suffix(',').unwrap_or(text.trim_end()).trim_end()
}

/// The type-start test for a parameter / typed member type: an
/// uppercase, `[`, or `(` start, a primitive / `Self` name, or a
/// reference type passing rule R.
fn type_start_passes(text: &str) -> bool {
    let text = text.trim();
    let Some(first) = text.chars().next() else {
        return false;
    };
    match first {
        'A'..='Z' | '[' | '(' => true,
        '&' => reference_type_passes(text),
        _ => match leading_identifier(text) {
            Some((ident, _)) => {
                let plain = ident.strip_prefix("r#").unwrap_or(&ident);
                PRIMITIVE_TYPES.contains(&plain)
            }
            None => false,
        },
    }
}

/// Rule R's named-type test (R1): the type is `str`, a primitive, or
/// `Self`, or an identifier / path whose final segment begins with an
/// uppercase ASCII letter and is not immediately followed by `(` (the
/// `Fn` family excepted: its `(` is trait-argument sugar).
fn named_type_passes(text: &str) -> bool {
    let text = text.trim();
    if text.is_empty() {
        return false;
    }
    let path_end = text
        .find(|c: char| c.is_whitespace() || c == '(')
        .unwrap_or(text.len());
    let path = &text[..path_end];
    let path_core = path.split('<').next().unwrap_or_default();
    if path_core.is_empty() {
        return false;
    }
    let final_segment = path_core.rsplit("::").next().unwrap_or_default();
    let Some((ident, ident_len)) = leading_identifier(final_segment) else {
        return false;
    };
    if ident_len != final_segment.len() {
        return false;
    }
    let plain = ident.strip_prefix("r#").unwrap_or(&ident);
    if PRIMITIVE_TYPES.contains(&plain) {
        return true;
    }
    if !plain.chars().next().unwrap().is_ascii_uppercase() {
        return false;
    }
    // Immediately after the path core: `(` is a call, not a type —
    // except for the `Fn` family's trait-argument sugar.
    match text[path_core.len()..].chars().next() {
        Some('(') => matches!(plain, "Fn" | "FnMut" | "FnOnce"),
        _ => true,
    }
}

/// Finds the end (exclusive byte index) of the balanced group opening
/// at the first character of `text`, counting `[ ] ( ) { } < >` and
/// ignoring the `>` of a `->`.
fn balanced_group_end(text: &str) -> Option<usize> {
    let bytes = text.as_bytes();
    let mut depth = 0i32;
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'-' if bytes.get(i + 1) == Some(&b'>') => {
                i += 2;
                continue;
            }
            b'[' | b'(' | b'{' | b'<' => depth += 1,
            b']' | b')' | b'}' | b'>' => {
                depth -= 1;
                if depth == 0 {
                    return Some(i + 1);
                }
                if depth < 0 {
                    return None;
                }
            }
            _ => {}
        }
        i += 1;
    }
    None
}

/// Whether `text` contains `target` at bracket depth 0.
fn contains_at_depth_zero(text: &str, target: u8) -> bool {
    let bytes = text.as_bytes();
    let mut depth = 0i32;
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'-' if bytes.get(i + 1) == Some(&b'>') => {
                i += 2;
                continue;
            }
            b'[' | b'(' | b'{' | b'<' => depth += 1,
            b']' | b')' | b'}' | b'>' => depth -= 1,
            c if c == target && depth == 0 => return true,
            _ => {}
        }
        i += 1;
    }
    false
}

/// Splits `text` at the first `target` occurring at bracket depth 0.
fn split_at_depth_zero(text: &str, target: u8) -> Option<(&str, &str)> {
    let bytes = text.as_bytes();
    let mut depth = 0i32;
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'-' if bytes.get(i + 1) == Some(&b'>') => {
                i += 2;
                continue;
            }
            b'[' | b'(' | b'{' | b'<' => depth += 1,
            b']' | b')' | b'}' | b'>' => depth -= 1,
            c if c == target && depth == 0 => {
                return Some((&text[..i], &text[i + 1..]));
            }
            _ => {}
        }
        i += 1;
    }
    None
}

/// Rule R3's bracket-type test: the bracket group is the whole of
/// `text`, its content is non-empty, and it is a slice type (content
/// passes the type-start test, no top-level comma) or an array type
/// (`Type ; length`).
fn bracket_type_passes(text: &str) -> bool {
    let text = text.trim();
    if !text.starts_with('[') {
        return false;
    }
    if balanced_group_end(text) != Some(text.len()) {
        return false;
    }
    let content = text[1..text.len() - 1].trim();
    if content.is_empty() {
        return false;
    }
    if let Some((element, length)) = split_at_depth_zero(content, b';') {
        let length = length.trim();
        type_start_passes(element.trim())
            && !length.is_empty()
            && length
                .chars()
                .next()
                .map(|c| c.is_ascii_digit() || c.is_ascii_alphabetic() || c == '_')
                .unwrap_or(false)
    } else {
        !contains_at_depth_zero(content, b',') && type_start_passes(content)
    }
}

/// Rule R: a reference type. Consumes the leading `&` (recursively),
/// an optional lifetime, and an optional `mut`, then requires the
/// remainder to be a named type (R1), a `dyn` / `impl` type (R2), or
/// a bracket type (R3). Reference *values* (`&[]`, `&some_var`,
/// `&(expr)`, calls) fail.
fn reference_type_passes(text: &str) -> bool {
    let mut rest = text.trim();
    if !rest.starts_with('&') {
        return false;
    }
    loop {
        if let Some(after) = rest.strip_prefix('&') {
            rest = after;
            continue;
        }
        if rest.starts_with('\'') {
            // A lifetime token must be followed by whitespace.
            let Some((_, len)) = leading_identifier(&rest[1..]) else {
                return false;
            };
            let after = &rest[1 + len..];
            if !after.starts_with(char::is_whitespace) {
                return false;
            }
            rest = after.trim_start();
            continue;
        }
        if rest.starts_with("mut")
            && rest[3..].starts_with(char::is_whitespace)
        {
            rest = rest[3..].trim_start();
            continue;
        }
        break;
    }
    let rest = rest.trim();
    if rest.starts_with('[') {
        return bracket_type_passes(rest);
    }
    for keyword in ["dyn", "impl"] {
        if let Some(after) = rest.strip_prefix(keyword) {
            if after.starts_with(char::is_whitespace) {
                return named_type_passes(after.trim_start());
            }
        }
    }
    named_type_passes(rest)
}

/// The TypeScript primitive / keyword types a typed member may use
/// where the Rust type test expects Rust spellings.
const TS_PRIMITIVE_TYPES: [&str; 11] = [
    "string", "number", "boolean", "null", "undefined", "any", "unknown", "object", "bigint",
    "symbol", "void",
];

/// Whether `text` is a TypeScript type expression for pattern 3b:
/// a union whose every member is a TypeScript primitive, an
/// uppercase-start named type, an array of one of those, or a quoted
/// literal — with a lone quoted literal rejected by the caller-side
/// rule encoded here (a single literal is a value shape, not a type
/// shape, unless it appears in a union).
fn typescript_type_passes(text: &str) -> bool {
    let text = text.trim();
    if text.is_empty() {
        return false;
    }
    let parts: Vec<&str> = text.split('|').collect();
    if parts.len() == 1 && (text.starts_with('"') || text.starts_with('\'')) {
        return false;
    }
    parts.iter().all(|part| {
        let part = part.trim().trim_end_matches("[]").trim();
        if part.starts_with('"') || part.starts_with('\'') {
            return part.len() >= 2;
        }
        match leading_identifier(part) {
            Some((ident, _)) => {
                TS_PRIMITIVE_TYPES.contains(&ident.as_str())
                    || ident
                        .chars()
                        .next()
                        .map(|c| c.is_ascii_uppercase())
                        .unwrap_or(false)
            }
            None => false,
        }
    })
}

/// Splits `text` at commas occurring at bracket depth 0.
fn split_top_level_commas(text: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let bytes = text.as_bytes();
    let mut depth = 0i32;
    let mut start = 0;
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'-' if bytes.get(i + 1) == Some(&b'>') => {
                i += 2;
                continue;
            }
            b'[' | b'(' | b'{' | b'<' => depth += 1,
            b']' | b')' | b'}' | b'>' => depth -= 1,
            b',' if depth == 0 => {
                parts.push(&text[start..i]);
                start = i + 1;
            }
            _ => {}
        }
        i += 1;
    }
    parts.push(&text[start..]);
    parts
}

/// The byte index of a single `:` at bracket depth 0 in `text` — one
/// that is not half of a `::` path separator — or `None`.
fn single_colon_index(text: &str) -> Option<usize> {
    let bytes = text.as_bytes();
    let mut depth = 0i32;
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'-' if bytes.get(i + 1) == Some(&b'>') => {
                i += 2;
                continue;
            }
            b'[' | b'(' | b'{' | b'<' => depth += 1,
            b']' | b')' | b'}' | b'>' => depth -= 1,
            b':' if depth == 0
                && bytes.get(i + 1) != Some(&b':')
                && (i == 0 || bytes[i - 1] != b':') =>
            {
                return Some(i);
            }
            _ => {}
        }
        i += 1;
    }
    None
}

/// Whether `text` is a type expression usable as an enum variant
/// payload: a named type (R1), a reference type (rule R), a bracket
/// type (R3), a tuple type whose elements all pass, or a `dyn` /
/// `impl` type over a named type. Value expressions fail: a variable
/// (`out`), a call (`Error::new()`), and a literal are not types.
fn variant_type_passes(text: &str) -> bool {
    let text = text.trim();
    if text.starts_with('[') {
        return bracket_type_passes(text);
    }
    if text.starts_with('(') {
        if balanced_group_end(text) != Some(text.len()) {
            return false;
        }
        let inner = text[1..text.len() - 1].trim();
        return inner.is_empty()
            || split_top_level_commas(inner)
                .iter()
                .all(|part| variant_payload_part_passes(part));
    }
    if text.starts_with('&') {
        return reference_type_passes(text);
    }
    for keyword in ["dyn", "impl"] {
        if let Some(after) = text.strip_prefix(keyword) {
            if after.starts_with(char::is_whitespace) {
                return named_type_passes(after.trim_start());
            }
        }
    }
    named_type_passes(text)
}

/// Whether one comma-separated part of an enum variant payload is
/// surface-shaped: a type, or a struct-style `field: Type` entry
/// whose type passes.
fn variant_payload_part_passes(part: &str) -> bool {
    let part = part.trim();
    if part.is_empty() {
        return false;
    }
    if let Some(idx) = single_colon_index(part) {
        let name = part[..idx].trim();
        let name_ok = match leading_identifier(name) {
            Some((_, len)) => len == name.len(),
            None => false,
        };
        return name_ok && variant_type_passes(part[idx + 1..].trim());
    }
    variant_type_passes(part)
}

/// Whether a variant's balanced payload group (delimiters included)
/// is type-shaped. An empty payload passes; a payload of value
/// expressions (`Ok(out)`, `Some(value)`) fails, because a tuple
/// variant declares types, never values.
fn variant_payload_passes(group: &str) -> bool {
    let content = group[1..group.len() - 1].trim();
    content.is_empty()
        || split_top_level_commas(content)
            .iter()
            .all(|part| variant_payload_part_passes(part))
}

/// Parses one indented line as a member of an enclosing public
/// declaration (method, named field, parameter / typed member, or
/// variant / bare member token), returning (name, key, signature).
/// Indentation is the enclosing-declaration signal: top-level items
/// sit at column 0, interior lines are indented.
fn parse_member_declaration(line: &str) -> Option<(String, String, String)> {
    if line == line.trim_start() {
        return None;
    }
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return None;
    }
    let normalized = normalized_line(trimmed);

    // Pattern 1: method — optional visibility, optional qualifiers,
    // then `fn`, a name, and `(`.
    {
        let rest = match strip_visibility_prefix(trimmed) {
            Some(after) => after,
            None => trimmed,
        };
        let mut tokens: Vec<&str> = rest.split_whitespace().collect();
        let mut start = 0;
        while start < tokens.len() {
            match tokens[start] {
                "async" | "unsafe" | "default" => start += 1,
                "const" if tokens.get(start + 1) == Some(&"fn") => start += 1,
                "extern" => {
                    start += 1;
                    if tokens
                        .get(start)
                        .map(|token| token.starts_with('"'))
                        .unwrap_or(false)
                    {
                        start += 1;
                    }
                }
                _ => break,
            }
        }
        tokens.drain(..start);
        if tokens.first() == Some(&"fn") {
            if let Some(name_token) = tokens.get(1) {
                if let Some((name, len)) = leading_identifier(name_token) {
                    let after_name = &name_token[len..];
                    if after_name.starts_with('(')
                        || (after_name.is_empty()
                            && tokens
                                .get(2)
                                .map(|token| token.starts_with('('))
                                .unwrap_or(false))
                    {
                        let signature = declaration_signature("fn", trimmed);
                        return Some((name.clone(), format!("fn:{name}"), signature));
                    }
                }
            }
        }
    }

    // Pattern 2: named field — bare `pub` then `<name>`:..
    if let Some(after) = strip_visibility_prefix(trimmed) {
        if let Some((name, len)) = leading_identifier(after) {
            let after_name = &after[len..];
            if after_name.starts_with(':') && !after_name.starts_with("::") {
                let signature = strip_trailing_comma(&normalized).to_string();
                return Some((name.clone(), format!("field:{name}"), signature));
            }
        }
    }

    // Pattern 3: parameter / typed member — `<name>: <Type>`.
    {
        let first_token = trimmed.split_whitespace().next().unwrap_or_default();
        if !["let", "return", "if", "for", "while", "match", "use"]
            .contains(&first_token)
        {
            if let Some((name, len)) = leading_identifier(trimmed) {
                let after_name = &trimmed[len..];
                if after_name.starts_with(':') && !after_name.starts_with("::") {
                    let type_part = strip_trailing_comma(after_name[1..].trim());
                    if !type_part.contains('=') && type_start_passes(type_part) {
                        let signature = strip_trailing_comma(&normalized).to_string();
                        return Some((name.clone(), format!("member:{name}"), signature));
                    }
                }
            }
        }
    }

    // Pattern 3b: TypeScript typed member — `<name>[?]: <TsType>` or an
    // interface method `<name>(<params>): <TsType>;`. The Rust pattern 3
    // type test rejects TypeScript primitives (`string`, `boolean`,
    // `number`), so members of an exported interface whose fields use
    // them extracted nothing. A lone quoted literal is NOT accepted as
    // a type here: it is indistinguishable from an object-literal value
    // (`principal: "alice",`), which the struct-literal guards exclude;
    // a union of literals (containing `|`) is accepted.
    {
        let first_token = trimmed.split_whitespace().next().unwrap_or_default();
        if !["let", "return", "if", "for", "while", "match", "use"].contains(&first_token) {
            if let Some((name, len)) = leading_identifier(trimmed) {
                let after_name = &trimmed[len..];
                let after_name = after_name.strip_prefix('?').unwrap_or(after_name);
                if let Some(type_part) = after_name.strip_prefix(':') {
                    let type_part = strip_trailing_comma(type_part.trim())
                        .trim_end_matches(';')
                        .trim();
                    if !type_part.contains('=') && typescript_type_passes(type_part) {
                        let signature = strip_trailing_comma(&normalized).to_string();
                        return Some((name.clone(), format!("member:{name}"), signature));
                    }
                }
                if after_name.starts_with('(') && trimmed.ends_with(';') {
                    if let Some(group_end) = balanced_group_end(after_name) {
                        if let Some(type_part) = after_name[group_end..]
                            .trim_start()
                            .strip_prefix(':')
                        {
                            let type_part = strip_trailing_comma(type_part.trim())
                                .trim_end_matches(';')
                                .trim();
                            if !type_part.contains('=')
                                && (typescript_type_passes(type_part)
                                    || type_start_passes(type_part))
                            {
                                let signature =
                                    strip_trailing_comma(&normalized).to_string();
                                return Some((name.clone(), format!("fn:{name}"), signature));
                            }
                        }
                    }
                }
            }
        }
    }

    // Pattern 4: variant / bare member token — a path or identifier,
    // optionally one balanced group, optionally a trailing comma.
    {
        let core = strip_trailing_comma(trimmed);
        if !core.contains(';') && !core.contains('=') {
            let group_start = core.find(['(', '{']);
            let (token, has_group) = match group_start {
                Some(idx) => (&core[..idx], true),
                None => (core, false),
            };
            let token = token.trim_end();
            let group_ok = if has_group {
                let group = &core[group_start.unwrap()..];
                let (open, close) = if group.starts_with('(') {
                    ('(', ')')
                } else {
                    ('{', '}')
                };
                let mut depth = 0i32;
                let mut ends_at_close = false;
                for (i, c) in group.char_indices() {
                    if c == open {
                        depth += 1;
                    } else if c == close {
                        depth -= 1;
                        if depth == 0 {
                            ends_at_close = i + c.len_utf8() == group.len();
                            break;
                        }
                    }
                }
                // A payload group must be type-shaped: a body
                // expression such as `Ok(out)` carries a value, and
                // a value can never be a variant payload.
                depth == 0 && ends_at_close && variant_payload_passes(group)
            } else {
                !token.contains(char::is_whitespace)
            };
            if group_ok && !token.is_empty() {
                let mut segments_ok = true;
                let mut final_segment = "";
                for segment in token.split("::") {
                    match leading_identifier(segment) {
                        Some((_, len)) if len == segment.len() => {
                            final_segment = segment;
                        }
                        _ => {
                            segments_ok = false;
                            break;
                        }
                    }
                }
                if segments_ok && !final_segment.is_empty() {
                    let raw = final_segment.starts_with("r#");
                    let plain = final_segment.strip_prefix("r#").unwrap_or(final_segment);
                    let keyword_ok = raw || !RUST_KEYWORDS.contains(&plain);
                    let uppercase_ok = plain
                        .chars()
                        .next()
                        .map(|c| c.is_ascii_uppercase())
                        .unwrap_or(false);
                    if keyword_ok && uppercase_ok {
                        let content = strip_trailing_comma(&normalized).to_string();
                        let key = format!("member:{content}");
                        return Some((final_segment.to_string(), key, content));
                    }
                }
            }
        }
    }

    None
}

/// Which non-code regions the blanking scanner erases. The
/// comments-only mode produces the intermediate reserved for the
/// pre-blanking extraction point (pipeline step 2 in
/// `extract_public_surface_declarations`): comments blanked, string
/// literal contents preserved, because a route registration's identity
/// is its path string literal.
#[derive(Clone, Copy, PartialEq, Eq)]
enum BlankMode {
    CommentsAndStrings,
    CommentsOnly,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum BlankState {
    Normal,
    LineComment,
    /// Nesting depth of `/* … */` (Rust block comments nest).
    BlockComment(u32),
    Str,
    /// Hash count of the opening `r#…#"` delimiter.
    RawString(usize),
    Template,
}

/// Blanks comments and string literals in `text`: every character of a
/// comment or string literal (delimiters included) becomes a space,
/// while every `\n` survives at its original offset, so the line-based
/// declaration parsers see the same line structure — and the same
/// leading-whitespace shape — with comment/string contents erased. A
/// declaration-shaped line inside a block comment or a multiline string
/// therefore extracts nothing. Unterminated constructs blank to the end
/// of input (fail-closed toward not extracting).
fn blank_comments_and_strings(text: &str) -> String {
    blank_non_code(text, BlankMode::CommentsAndStrings)
}

/// Blanks comments only, preserving string literal contents. String
/// states are still tracked, so comment markers inside a string literal
/// start no comment here either. This is the intermediate the reserved
/// pre-blanking extraction point consumes; it is a by-product of the
/// same scan, not a second scanner.
fn blank_comments_only(text: &str) -> String {
    blank_non_code(text, BlankMode::CommentsOnly)
}

fn blank_non_code(text: &str, mode: BlankMode) -> String {
    let bytes = text.as_bytes();
    let blank_strings = mode == BlankMode::CommentsAndStrings;
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut state = BlankState::Normal;
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        match state {
            BlankState::Normal => {
                if b == b'/' && bytes.get(i + 1) == Some(&b'/') {
                    out.extend_from_slice(b"  ");
                    i += 2;
                    state = BlankState::LineComment;
                } else if b == b'/' && bytes.get(i + 1) == Some(&b'*') {
                    out.extend_from_slice(b"  ");
                    i += 2;
                    state = BlankState::BlockComment(1);
                } else if b == b'"' {
                    out.push(if blank_strings { b' ' } else { b });
                    i += 1;
                    state = BlankState::Str;
                } else if b == b'`' {
                    out.push(if blank_strings { b' ' } else { b });
                    i += 1;
                    state = BlankState::Template;
                } else if b == b'\'' {
                    match char_literal_len(text, i) {
                        Some(len) => {
                            if blank_strings {
                                out.extend(std::iter::repeat_n(b' ', len));
                            } else {
                                out.extend_from_slice(&bytes[i..i + len]);
                            }
                            i += len;
                        }
                        None => {
                            // A lifetime (`'a`, `'static`): not a
                            // literal; emit and stay in Normal.
                            out.push(b);
                            i += 1;
                        }
                    }
                } else if b == b'r' || (b == b'b' && bytes.get(i + 1) == Some(&b'r')) {
                    match raw_string_open(bytes, i) {
                        Some((prefix_len, hashes)) => {
                            if blank_strings {
                                out.extend(std::iter::repeat_n(b' ', prefix_len));
                            } else {
                                out.extend_from_slice(&bytes[i..i + prefix_len]);
                            }
                            i += prefix_len;
                            state = BlankState::RawString(hashes);
                        }
                        None => {
                            out.push(b);
                            i += 1;
                        }
                    }
                } else {
                    out.push(b);
                    i += 1;
                }
            }
            BlankState::LineComment => {
                if b == b'\n' {
                    out.push(b'\n');
                    state = BlankState::Normal;
                } else {
                    out.push(b' ');
                }
                i += 1;
            }
            BlankState::BlockComment(depth) => {
                if b == b'/' && bytes.get(i + 1) == Some(&b'*') {
                    out.extend_from_slice(b"  ");
                    i += 2;
                    state = BlankState::BlockComment(depth + 1);
                } else if b == b'*' && bytes.get(i + 1) == Some(&b'/') {
                    out.extend_from_slice(b"  ");
                    i += 2;
                    state = if depth == 1 {
                        BlankState::Normal
                    } else {
                        BlankState::BlockComment(depth - 1)
                    };
                } else if b == b'\n' {
                    out.push(b'\n');
                    i += 1;
                } else {
                    out.push(b' ');
                    i += 1;
                }
            }
            BlankState::Str => {
                if b == b'\\' {
                    out.push(if blank_strings { b' ' } else { b });
                    i += 1;
                    if i < bytes.len() {
                        let escaped = bytes[i];
                        if escaped == b'\n' {
                            out.push(b'\n');
                        } else {
                            out.push(if blank_strings { b' ' } else { escaped });
                        }
                        i += 1;
                    }
                } else if b == b'"' {
                    out.push(if blank_strings { b' ' } else { b });
                    i += 1;
                    state = BlankState::Normal;
                } else if b == b'\n' {
                    out.push(b'\n');
                    i += 1;
                } else {
                    out.push(if blank_strings { b' ' } else { b });
                    i += 1;
                }
            }
            BlankState::RawString(hashes) => {
                if b == b'"' {
                    let mut following = 0;
                    while bytes.get(i + 1 + following) == Some(&b'#') {
                        following += 1;
                    }
                    if following >= hashes {
                        for k in 0..1 + hashes {
                            let c = bytes[i + k];
                            out.push(if blank_strings { b' ' } else { c });
                        }
                        i += 1 + hashes;
                        state = BlankState::Normal;
                    } else {
                        out.push(if blank_strings { b' ' } else { b });
                        i += 1;
                    }
                } else if b == b'\n' {
                    out.push(b'\n');
                    i += 1;
                } else {
                    out.push(if blank_strings { b' ' } else { b });
                    i += 1;
                }
            }
            BlankState::Template => {
                if b == b'\\' {
                    out.push(if blank_strings { b' ' } else { b });
                    i += 1;
                    if i < bytes.len() {
                        let escaped = bytes[i];
                        if escaped == b'\n' {
                            out.push(b'\n');
                        } else {
                            out.push(if blank_strings { b' ' } else { escaped });
                        }
                        i += 1;
                    }
                } else if b == b'`' {
                    out.push(if blank_strings { b' ' } else { b });
                    i += 1;
                    state = BlankState::Normal;
                } else if b == b'\n' {
                    out.push(b'\n');
                    i += 1;
                } else {
                    out.push(if blank_strings { b' ' } else { b });
                    i += 1;
                }
            }
        }
    }
    // Blanking only substitutes ASCII spaces for non-newline bytes and
    // copies Normal-state bytes verbatim, so the output is valid UTF-8
    // whenever the input is.
    String::from_utf8_lossy(&out).into_owned()
}

/// If the `'` at byte `start` opens a char literal, returns its byte
/// length: `'\X'` (backslash, one char, closing quote) or `'c'` (one
/// char that is not `'` or `\n`, closing quote). Anything else — `'a`,
/// `'static` — is a lifetime and returns `None`, so the scanner stays in
/// Normal state rather than blanking real code to the next quote.
fn char_literal_len(text: &str, start: usize) -> Option<usize> {
    let rest = &text[start + 1..];
    let mut chars = rest.char_indices();
    let (_, first) = chars.next()?;
    if first == '\\' {
        let (_, _escaped) = chars.next()?;
        let (end, closing) = chars.next()?;
        (closing == '\'').then_some(1 + end + 1)
    } else if first == '\'' || first == '\n' {
        None
    } else {
        let (end, closing) = chars.next()?;
        (closing == '\'').then_some(1 + end + 1)
    }
}

/// If a raw string opens at byte `start` — an `r` (optionally preceded
/// by a `b` byte-string prefix that is not part of a longer identifier)
/// followed by zero or more `#` and a `"` — returns the opener's byte
/// length and its hash count. The `r` must not be preceded by an
/// identifier character, so identifiers ending in `r` never qualify;
/// raw identifiers (`r#type`) fail the closing-quote check.
fn raw_string_open(bytes: &[u8], start: usize) -> Option<(usize, usize)> {
    let is_ident = |c: u8| c.is_ascii_alphanumeric() || c == b'_';
    if start > 0 && is_ident(bytes[start - 1]) {
        return None;
    }
    let r_idx = if bytes[start] == b'r' {
        start
    } else {
        start + 1
    };
    let mut j = r_idx + 1;
    while bytes.get(j) == Some(&b'#') {
        j += 1;
    }
    if bytes.get(j) != Some(&b'"') {
        return None;
    }
    Some((j + 1 - start, j - r_idx - 1))
}

/// Normalizes a TypeScript export line into its public signature,
/// mirroring `declaration_signature`'s philosophy: a `function`
/// signature ends before the body brace; `class` / `interface` /
/// `enum` / `namespace` keep the whole line when its braces balance
/// on the line (inline members are surface) and otherwise keep only
/// the header before the first `{`; a `type` alias keeps the whole
/// line minus the trailing `;`, because the aliased definition is
/// its surface; a `const` / `let` / `var` keeps the whole line minus
/// the trailing `;`, because an exported value is its contract —
/// except that an arrow-function value keeps only the text up to
/// the first `{` after the `=>` when one exists on the line, so a
/// same-line body edit does not fire. Re-export lines keep the
/// whole line minus the trailing `;`, so the source path (which
/// the signature carries) stays precise.
fn typescript_signature(kind: &str, line: &str) -> String {
    let normalized = normalized_line(line);
    let strip_trailing_semicolon = |text: &str| {
        text.trim_end()
            .trim_end_matches(';')
            .trim_end()
            .to_string()
    };
    match kind {
        "function" => {
            let head = match normalized.find('{') {
                Some(idx) => &normalized[..idx],
                None => &normalized[..],
            };
            strip_trailing_semicolon(head.trim_end())
        }
        "class" | "interface" | "enum" | "namespace" => {
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
        "type" => strip_trailing_semicolon(&normalized),
        "const" | "let" | "var" => {
            if let Some(arrow) = normalized.find("=>") {
                match normalized[arrow + 2..].find('{') {
                    Some(rel) => normalized[..arrow + 2 + rel].trim_end().to_string(),
                    None => normalized,
                }
            } else {
                strip_trailing_semicolon(&normalized)
            }
        }
        _ => strip_trailing_semicolon(&normalized),
    }
}

/// Parses one line as a TypeScript `export` declaration, returning
/// its (declared name, signature) pairs, or `None` when the line
/// does not begin an export. Recognition runs on the blanked line
/// (comment/string contents are already erased there, so an
/// `export` shape inside one extracts nothing); the signature is
/// taken from the original line, because an exported `const`'s
/// string value and a re-export's source path are surface that
/// blanking erases. Modifiers (`default` / `declare` / `abstract` /
/// `async`, in any order) precede the kind; an anonymous default
/// export (name token `(` or `{`) inserts nothing, as do shapes
/// outside the kind list (`export = …`, `export as namespace …`).
fn parse_typescript_exports(
    blanked_line: &str,
    original_line: &str,
) -> Option<Vec<(String, String)>> {
    const TS_KINDS: [&str; 9] = [
        "function", "class", "interface", "type", "const", "let", "var", "enum", "namespace",
    ];
    let trimmed = blanked_line.trim();
    let rest = trimmed.strip_prefix("export")?;
    if !rest.starts_with(char::is_whitespace) {
        return None;
    }
    let tokens: Vec<&str> = trimmed.split_whitespace().collect();
    let mut idx = 1;
    while idx < tokens.len()
        && ["default", "declare", "abstract", "async"].contains(&tokens[idx])
    {
        idx += 1;
    }
    let Some(next) = tokens.get(idx) else {
        return Some(Vec::new());
    };
    if next.starts_with('{') {
        // Re-export list: `export { A, B as C, type D } from "…";`
        // (the `from` clause is optional). A group with no closing
        // brace on this line is a multiline header; its member
        // lines are caught by the member parser instead.
        let (Some(open), Some(close)) = (trimmed.find('{'), trimmed.rfind('}')) else {
            return Some(Vec::new());
        };
        if close <= open {
            return Some(Vec::new());
        }
        let signature = typescript_signature("reexport", original_line);
        let mut out = Vec::new();
        for member in trimmed[open + 1..close].split(',') {
            let member = member.trim();
            if member.is_empty() {
                continue;
            }
            let member = member.strip_prefix("type ").unwrap_or(member).trim();
            let name = if let Some((_, alias)) = member.rsplit_once(" as ") {
                leading_identifier(alias.trim()).map(|(name, _)| name)
            } else {
                leading_identifier(member).map(|(name, _)| name)
            };
            if let Some(name) = name {
                if !name.is_empty() {
                    out.push((name, signature.clone()));
                }
            }
        }
        return Some(out);
    }
    if next.starts_with('*') {
        // Star re-export: `export * as ns from "…";` names `ns`;
        // a bare `export * from "./module";` names the module
        // path's final segment minus its extension.
        let signature = typescript_signature("reexport", original_line);
        if tokens.get(idx + 1) == Some(&"as") {
            if let Some(name_token) = tokens.get(idx + 2) {
                if let Some((name, _)) = leading_identifier(name_token) {
                    return Some(vec![(name, signature)]);
                }
            }
            return Some(Vec::new());
        }
        let original = original_line.trim();
        let quote = original.find(['"', '\'']);
        let Some(quote) = quote else {
            return Some(Vec::new());
        };
        let quote_char = original[quote..].chars().next().unwrap_or('"');
        let path = &original[quote + 1..];
        let Some(end) = path.find(quote_char) else {
            return Some(Vec::new());
        };
        let final_segment = path[..end].rsplit('/').next().unwrap_or_default();
        let stem = match final_segment.rfind('.') {
            Some(dot) if dot > 0 => &final_segment[..dot],
            _ => final_segment,
        };
        return Some(match leading_identifier(stem) {
            Some((name, _)) if !name.is_empty() => vec![(name, signature)],
            _ => Vec::new(),
        });
    }
    if !TS_KINDS.contains(next) {
        return Some(Vec::new());
    }
    let kind = *next;
    let Some(name_token) = tokens.get(idx + 1) else {
        return Some(Vec::new());
    };
    if name_token.starts_with('(') || name_token.starts_with('{') {
        return Some(Vec::new());
    }
    let Some((name, _)) = leading_identifier(name_token) else {
        return Some(Vec::new());
    };
    if name.is_empty() {
        return Some(Vec::new());
    }
    let signature = typescript_signature(kind, original_line);
    Some(vec![(name, signature)])
}

/// Parses one HTTP route registration line into its surface key and
/// signature. Two shapes, both matched on the
/// comments-blanked-but-strings-preserved intermediate (a route's
/// identity is its path string literal, which full blanking erases):
/// an inline `.route("/path", get(...))` line, whose methods are the
/// whole-word `get` / `post` / `put` / `delete` / `patch` / `head` /
/// `options` calls in the remainder after the path literal (a token
/// immediately following `.`, or the first such token in the
/// remainder, immediately followed by `(`); and a split route-path
/// line — an indented line that is exactly a `"/…"` string literal
/// followed by `,` — which carries an empty method set. Handler
/// identifiers are deliberately not part of the signature: the
/// documented HTTP surface is (path, method), so a handler swap behind
/// an unchanged registration is an implementation change.
fn parse_http_route_registration(line: &str) -> Option<(String, String)> {
    let trimmed = line.trim();
    if let Some(route_idx) = trimmed.find(".route(") {
        let after = trimmed[route_idx + ".route(".len()..].trim_start();
        if let Some(rest) = after.strip_prefix('"') {
            // No escape processing: a literal containing `\"` is
            // truncated at the escape (recorded residual).
            let close = rest.find('"')?;
            let path = &rest[..close];
            let remainder = &rest[close + 1..];
            let mut methods: std::collections::BTreeSet<&str> = std::collections::BTreeSet::new();
            let bytes = remainder.as_bytes();
            let mut i = 0;
            while i < bytes.len() {
                if bytes[i].is_ascii_alphabetic() || bytes[i] == b'_' {
                    let start = i;
                    while i < bytes.len()
                        && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_')
                    {
                        i += 1;
                    }
                    let token = &remainder[start..i];
                    let is_method = matches!(
                        token,
                        "get" | "post" | "put" | "delete" | "patch" | "head" | "options"
                    );
                    let followed_by_paren = bytes.get(i) == Some(&b'(');
                    let preceded_by_dot = start > 0 && bytes[start - 1] == b'.';
                    if is_method && followed_by_paren && (preceded_by_dot || methods.is_empty())
                    {
                        methods.insert(token);
                    }
                } else {
                    i += 1;
                }
            }
            let key = format!("route:{path}");
            let signature = if methods.is_empty() {
                format!("route:{path}:-")
            } else {
                format!(
                    "route:{path}:{}",
                    methods.into_iter().collect::<Vec<_>>().join(",")
                )
            };
            return Some((key, signature));
        }
    }
    // Split route-path line: indented, sole string literal whose
    // contents begin with `/`, followed by a comma.
    let indented = line.starts_with(' ') || line.starts_with('\t');
    if indented && trimmed.starts_with('"') && trimmed.ends_with(',') {
        let inner = &trimmed[1..trimmed.len() - 1];
        if let Some(path) = inner.strip_suffix('"') {
            if path.starts_with('/') && !path.contains('"') {
                let key = format!("route:{path}");
                let signature = format!("route:{path}:-");
                return Some((key, signature));
            }
        }
    }
    None
}

/// Extracts public surface declarations from source text or diff lines:
/// everything `extract_public_surface_identifiers` returns, plus public item
/// declarations (bare-`pub` items, with optional async / unsafe /
/// const / default qualifiers, over struct/enum/trait/fn/type/const/static/
/// mod/use/union). For `use`, the declared names come from the
/// use-tree parser: every member of a grouped use, honouring `as`
/// renames, nested groups, `self`, and globs. Declaration parsing runs
/// on text whose comments and string literals have been blanked, so
/// declaration-shaped text inside a comment or string extracts nothing;
/// the SS7 identifiers are taken from the original text first, because
/// SS7 reads identifiers out of string contents.
pub fn extract_public_surface_declarations(text: &str) -> std::collections::BTreeSet<String> {
    // Pipeline step 1: SS7 identifiers, from the ORIGINAL text (SS7
    // extracts identifiers from string contents, which step 3 erases).
    let mut decls: std::collections::BTreeSet<String> = extract_public_surface_identifiers(text)
        .into_iter()
        .collect();
    // Pipeline step 2: pre-blanking extraction point. Route
    // registrations are extracted here, from the
    // comments-blanked-but-strings-preserved intermediate produced by
    // `blank_comments_only`, because a route's identity is a string
    // literal that step 3 erases. Nothing else may use this slot
    // without a design note.
    let route_text = blank_comments_only(text);
    for line in route_text.lines() {
        if let Some((key, _signature)) = parse_http_route_registration(line) {
            decls.insert(key);
        }
    }
    // Pipeline step 3: blank comments and string literals.
    let cleaned = blank_comments_and_strings(text);
    // A restricted-visibility item re-exported by a bare `pub use` in
    // the same text is public surface transitively: the use line is
    // extracted, so the definition behind it must be too, or its own
    // declaration changes would register nothing.
    let reexported = reexported_source_names(&cleaned);
    // Pipeline step 4: all declaration parsing runs on the blanked text.
    // The TypeScript export branch runs here too, on the blanked line
    // (its signatures are taken from the corresponding original line,
    // which blanking's line-structure preservation keeps aligned).
    let mut test_scope = TestScopeTracker::new();
    for (line, original_line) in cleaned.lines().zip(text.lines()) {
        // Test-only scopes extract nothing (see TestScopeTracker).
        let indent = line.len() - line.trim_start().len();
        if test_scope.suppresses(indent, line) {
            continue;
        }
        if let Some(pairs) = parse_typescript_exports(line, original_line) {
            for (name, _signature) in pairs {
                decls.insert(name);
            }
        } else if let Some(pairs) = parse_use_declarations(line) {
            for (name, _full_path) in pairs {
                decls.insert(name);
            }
        } else if let Some((name, _kind)) = parse_public_surface_declaration(line) {
            decls.insert(name);
        } else if let Some((name, _kind)) = parse_restricted_surface_declaration(line)
            .filter(|(name, _kind)| reexported.contains(name))
        {
            decls.insert(name);
        }
        if let Some((name, _key, _signature)) = parse_member_declaration(line) {
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

/// Parses one line as a fragment of an MCP tool `inputSchema`, returning
/// its (leaf, signature) pair. The leaf is the entry's own name — a
/// property name, `required`, or `type` — which the caller qualifies
/// with the owning tool and the nested property path (see
/// `mcp_schema_key`): entries used to aggregate under shared leaves, so
/// a change in one tool cancelled against the identical entry in
/// another. Three shapes, all matched on the original line (schema keys
/// and values live inside string literals that the blanking pipeline
/// erases): a single-line property carrying its type
/// (`"query": { "type": "string", ... }`, with the description
/// deliberately excluded so a description-only edit does not fire); a
/// standalone `"type": "<t>"` line (a property whose value object
/// opened on a previous line, or the schema's own type); and a
/// `"required": [...]` line, whose array contents are the signature.
/// Property lines without a same-line type and non-schema JSON lines
/// (`"name":`, `"description":`, `"properties": {`) extract nothing.
fn parse_mcp_schema_line(line: &str) -> Option<(String, String)> {
    let trimmed = line.trim();
    let rest = trimmed.strip_prefix('"')?;
    let (key, after) = rest.split_once('"')?;
    if key.is_empty() || !key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return None;
    }
    let after = after.trim_start().strip_prefix(':')?.trim_start();
    if key == "required" {
        let end = after.find(']')?;
        return Some(("required".to_string(), normalized_line(&after[..=end])));
    }
    if key == "type" {
        let value = after.strip_prefix('"')?;
        let end = value.find('"')?;
        return Some(("type".to_string(), value[..end].to_string()));
    }
    if after.starts_with('{') {
        let type_idx = after.find("\"type\":")?;
        let value = after[type_idx + "\"type\":".len()..]
            .trim_start()
            .strip_prefix('"')?;
        let end = value.find('"')?;
        return Some((key.to_string(), format!("{key}:{}", &value[..end])));
    }
    None
}

/// Composes the signature-map key for one MCP schema entry from its
/// owning tool, its nested property path (the property names enclosing
/// it, outermost first, with the structural `inputSchema` /
/// `properties` levels dropped), and its leaf. A contextless entry
/// (a lone schema line, the common -U0 diff shape) keeps the legacy
/// bare key `mcp-schema:<leaf>`.
fn mcp_schema_key(tool: Option<&str>, path: &[String], leaf: &str) -> String {
    let mut key = "mcp-schema:".to_string();
    if let Some(tool) = tool {
        key.push_str(tool);
        key.push(':');
    }
    if !path.is_empty() {
        key.push_str(&path.join("."));
        key.push('.');
    }
    key.push_str(leaf);
    key
}

/// The `"key"` opening a JSON-ish line (`"name": ...`), if any: the
/// first quoted token, when a colon follows it.
fn json_line_key(line: &str) -> Option<String> {
    let rest = line.trim().strip_prefix('"')?;
    let (key, after) = rest.split_once('"')?;
    if key.is_empty() || !after.trim_start().starts_with(':') {
        return None;
    }
    Some(key.to_string())
}

/// Parses one line as a clap argument attribute (`#[arg(...)]`),
/// returning the normalized attribute contents. An option's long/short
/// spelling, requiredness, and default are CLI surface, so the whole
/// attribute is the signature; it is matched on the original line
/// because a `default_value` string literal is part of that surface
/// and the blanking pipeline would erase it. All attributes share one
/// key: changing any one of them makes the added and removed signature
/// sets for the key differ.
fn parse_clap_arg_attribute(line: &str) -> Option<String> {
    let trimmed = line.trim();
    let inner = trimmed.strip_prefix("#[arg(")?.strip_suffix(")]")?;
    let inner = normalized_line(inner);
    if inner.is_empty() {
        None
    } else {
        Some(inner)
    }
}

/// Parses one line as a serde attribute (`#[serde(...)]`), returning
/// the normalized attribute contents. Serde container / field /
/// variant attributes (`tag`, `rename`, `rename_all`, `flatten`, …)
/// determine the serialized wire format, so the whole attribute is
/// surface. It is matched on the comments-blanked-but-strings-
/// preserved line, because its argument values are string literals
/// the full blanking pipeline erases — and a serde-shaped text inside
/// a comment must not count. The attribute may share its line with
/// the declaration it annotates, so the closing `)]` is located
/// rather than required at end of line.
fn parse_serde_attribute(line: &str) -> Option<String> {
    let inner_start = line.find("#[serde(")? + "#[serde(".len();
    let inner_end = line[inner_start..].find(")]")? + inner_start;
    let inner = normalized_line(&line[inner_start..inner_end]);
    if inner.is_empty() {
        None
    } else {
        Some(inner)
    }
}

/// Parses one line as a clap command attribute (`#[command(...)]`),
/// returning the normalized attribute contents. A command's name,
/// alias / visible_alias, about text, and subcommand wiring are CLI
/// surface, so the whole attribute is the signature; SS7 extracts
/// only the `name = "..."` value, so an alias-only change otherwise
/// produced identical maps. Matched on the comments-blanked-but-
/// strings-preserved line, like serde, because the argument values
/// are string literals the full blanking pipeline erases — and a
/// command-shaped text inside a comment must not count. The closing
/// `)]` is located rather than required at end of line, since the
/// attribute may share its line with the declaration it annotates.
fn parse_command_attribute(line: &str) -> Option<String> {
    let inner_start = line.find("#[command(")? + "#[command(".len();
    let inner_end = line[inner_start..].find(")]")? + inner_start;
    let inner = normalized_line(&line[inner_start..inner_end]);
    if inner.is_empty() {
        None
    } else {
        Some(inner)
    }
}

/// Parses one line as an `impl` block header, returning the name of
/// the type the block implements methods for (`impl Foo`,
/// `impl<T> Foo<T>`, `impl Trait for Foo` — the target type in each
/// case). Used only to attribute member signatures to their enclosing
/// type; the impl block itself is not a declaration kind the surface
/// extractor records.
fn parse_impl_enclosing(line: &str) -> Option<String> {
    let trimmed = line.trim();
    let rest = trimmed.strip_prefix("impl")?;
    if !rest.starts_with(|c: char| c.is_whitespace() || c == '<') {
        return None;
    }
    let head = rest.split('{').next()?.trim();
    // `impl Trait for Type`: the enclosing type follows `for`.
    let target = match head.rsplit_once(" for ") {
        Some((_, ty)) => ty.trim(),
        None => head,
    };
    // Skip a leading generic parameter list (`impl<T> Foo`).
    let target = if target.starts_with('<') {
        target[balanced_group_end(target)?..].trim()
    } else {
        target
    };
    // Drop the type's own generic arguments (`Foo<T>`).
    let target = target.split('<').next()?.trim();
    let token = target.split_whitespace().next()?;
    let segment = token.rsplit("::").next()?;
    let (name, len) = leading_identifier(segment)?;
    if len != segment.len() {
        return None;
    }
    Some(name)
}

/// Whether a trimmed line is a `#[cfg(test)]` attribute (a compound
/// `#[cfg(test, ...)]` included).
fn is_cfg_test_attribute(trimmed: &str) -> bool {
    match trimmed.strip_prefix("#[cfg(test") {
        Some(rest) => rest.starts_with(')') || rest.starts_with(','),
        None => false,
    }
}

/// Whether a trimmed line is a test-function attribute: `#[test]`, or
/// a path-qualified spelling whose final segment is `test`
/// (`#[tokio::test]`, `#[actix_rt::test]`, ...).
fn is_test_fn_attribute(trimmed: &str) -> bool {
    let Some(inner) = trimmed.strip_prefix("#[") else {
        return false;
    };
    let path = inner.split(['(', ']']).next().unwrap_or_default().trim();
    path.rsplit("::").next().unwrap_or_default().trim() == "test"
}

/// Whether a trimmed line opens a conventional test module without a
/// bare-`pub` qualifier (`mod tests {` / `mod test {`): such a module
/// is private, so its contents are test-only even when no
/// `#[cfg(test)]` precedes it.
fn is_test_module_header(trimmed: &str) -> bool {
    let rest = trimmed
        .strip_prefix("pub(crate) ")
        .or_else(|| trimmed.strip_prefix("pub(super) "))
        .unwrap_or(trimmed);
    ["mod tests", "mod test"].iter().any(|prefix| {
        rest.strip_prefix(prefix)
            .map(|after| {
                after.starts_with(|c: char| c.is_whitespace() || c == '{' || c == ';')
            })
            .unwrap_or(false)
    })
}

/// Tracks test-only scopes while scanning a file top to bottom, so
/// member/declaration extraction can skip them: the member parser
/// accepts any indented `fn` line, so without this a test function
/// (or a helper inside a test module) entered the public surface and
/// a test-only rename demanded a documentation update. A scope opens
/// at a `#[cfg(test)]` item, a `#[test]` function, or a private
/// `mod tests` header, and ends at the first non-blank line at or
/// above the opening line's indentation. Legitimate member shapes
/// outside test scopes — trait methods, inherent-impl helpers,
/// trait-impl methods — are untouched: suppression is keyed to the
/// test markers, never to visibility alone.
struct TestScopeTracker {
    pending_cfg_test: bool,
    pending_test_fn: bool,
    scope_indent: Option<usize>,
}

impl TestScopeTracker {
    fn new() -> Self {
        TestScopeTracker {
            pending_cfg_test: false,
            pending_test_fn: false,
            scope_indent: None,
        }
    }

    /// Feeds one blanked line (with its indentation); returns true
    /// when the line is test-only and must extract nothing.
    fn suppresses(&mut self, indent: usize, line: &str) -> bool {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            return false;
        }
        if let Some(scope) = self.scope_indent {
            if indent > scope {
                return true;
            }
            self.scope_indent = None;
        }
        if is_cfg_test_attribute(trimmed) {
            self.pending_cfg_test = true;
            return false;
        }
        if is_test_fn_attribute(trimmed) {
            self.pending_test_fn = true;
            return false;
        }
        if trimmed.starts_with("#[") {
            // Another attribute stacked between the test marker and
            // its item: the marker stays pending.
            return false;
        }
        let opens_block = line.matches('{').count() > line.matches('}').count();
        if self.pending_cfg_test || self.pending_test_fn {
            self.pending_cfg_test = false;
            self.pending_test_fn = false;
            if opens_block {
                self.scope_indent = Some(indent);
            }
            return true;
        }
        if is_test_module_header(trimmed) && opens_block {
            self.scope_indent = Some(indent);
        }
        false
    }
}

/// Parses one line as a container header of any visibility — a bare
/// `pub` item, a restricted-visibility item, or a private item
/// (`enum Commands {`, `mod tests {`) — returning its (name, kind)
/// when the kind is a container kind. Scope tracking needs private
/// containers too: a clap command enum is typically not `pub`, but
/// its variants and fields are still CLI surface, and member
/// qualification must know the enum encloses them. Extraction of the
/// container itself stays visibility-gated by the declaration
/// parsers; this parser only names the scope.
fn parse_any_container_header(line: &str) -> Option<(String, String)> {
    const CONTAINER_KINDS: [&str; 5] = ["struct", "enum", "trait", "union", "mod"];
    for parsed in [
        parse_public_surface_declaration(line),
        parse_restricted_surface_declaration(line),
    ]
    .into_iter()
    .flatten()
    {
        let (name, kind) = parsed;
        if CONTAINER_KINDS.contains(&kind.as_str()) {
            return Some((name, kind));
        }
    }
    let trimmed = line.trim();
    let mut tokens = trimmed.split_whitespace();
    let kind = tokens.next()?;
    if !CONTAINER_KINDS.contains(&kind) {
        return None;
    }
    let name_token = tokens.next()?;
    let (name, _len) = leading_identifier(name_token)?;
    Some((name, kind.to_string()))
}

/// Parses one indented line as a multiline enum variant header: an
/// uppercase variant name followed by a `{` (struct variant) or `(`
/// (tuple variant) payload that does NOT close on the same line. The
/// member parser's variant pattern requires the payload group to end
/// at its close on the header line, so these headers extract nothing
/// there; this parser recognizes the header alone, returning
/// (name, key, signature) in the member parser's shape.
fn parse_multiline_variant_header(line: &str) -> Option<(String, String, String)> {
    if line == line.trim_start() {
        return None;
    }
    let trimmed = line.trim();
    let (name, len) = leading_identifier(trimmed)?;
    let plain = name.strip_prefix("r#").unwrap_or(&name);
    if RUST_KEYWORDS.contains(&plain)
        || !plain
            .chars()
            .next()
            .map(|c| c.is_ascii_uppercase())
            .unwrap_or(false)
    {
        return None;
    }
    let rest = trimmed[len..].trim_start();
    let (open, close) = if rest.starts_with('{') {
        ('{', '}')
    } else if rest.starts_with('(') {
        ('(', ')')
    } else {
        return None;
    };
    if rest.matches(open).count() <= rest.matches(close).count() {
        return None;
    }
    let signature = normalized_line(trimmed);
    Some((name.clone(), format!("member:{name}"), signature))
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
    // Same pipeline as `extract_public_surface_declarations`: SS7
    // identifiers from the original text, then all declaration parsing
    // on the comment/string-blanked text.
    for ident in extract_public_surface_identifiers(text) {
        signatures.entry(ident.clone()).or_default().insert(ident);
    }
    // MCP input schemas, from the original text alongside SS7 (their
    // keys and values are string contents): a tool's parameter types
    // and required set are user-facing API surface, so a schema-only
    // change to an existing tool must register even though the tool
    // name line itself is unchanged.
    // The blanked text is computed here (and reused below) because
    // the schema scan tracks object nesting by counting braces, and
    // braces inside string literals (a description quoting a JSON
    // example) must not move the nesting.
    let cleaned = blank_comments_and_strings(text);
    // The tool whose schema is being scanned (`"name": "<tool>"` most
    // recently seen), and the stack of (indent, key) objects enclosing
    // the current line. Schema keys are qualified with both, so an
    // entry shared by two tools — or by two properties of one tool —
    // no longer cancels a change to just one of them.
    let mut mcp_tool: Option<String> = None;
    let mut mcp_stack: Vec<(usize, String)> = Vec::new();
    for (line, blanked) in text.lines().zip(cleaned.lines()) {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("\"name\": \"") {
            if let Some(end) = rest.find('"') {
                if end > 0 {
                    mcp_tool = Some(rest[..end].to_string());
                    mcp_stack.clear();
                }
            }
        }
        let indent = line.len() - line.trim_start().len();
        if !trimmed.is_empty() {
            while mcp_stack
                .last()
                .map(|(open_indent, _)| *open_indent >= indent)
                .unwrap_or(false)
            {
                mcp_stack.pop();
            }
        }
        if let Some((leaf, signature)) = parse_mcp_schema_line(line) {
            let path: Vec<String> = mcp_stack
                .iter()
                .map(|(_, key)| key.clone())
                .filter(|key| !key.is_empty() && key != "properties" && key != "inputSchema")
                .collect();
            let key = mcp_schema_key(mcp_tool.as_deref(), &path, &leaf);
            signatures.entry(key).or_default().insert(signature);
        }
        if let Some(attribute) = parse_clap_arg_attribute(line) {
            signatures
                .entry("clap-arg".to_string())
                .or_default()
                .insert(attribute);
        }
        let opens = blanked.matches('{').count() as i32;
        let closes = blanked.matches('}').count() as i32;
        if opens > closes {
            mcp_stack.push((indent, json_line_key(line).unwrap_or_default()));
        }
    }
    // Pipeline step 2, as in `extract_public_surface_declarations`:
    // route registrations from the comments-blanked-but-strings-
    // preserved intermediate, keyed `route:<path>` with the signature
    // carrying the sorted method set.
    let route_text = blank_comments_only(text);
    for line in route_text.lines() {
        if let Some((key, signature)) = parse_http_route_registration(line) {
            signatures.entry(key).or_default().insert(signature);
        }
        // Clap command attributes enter the shared `command` key
        // unconditionally, following the clap-arg pattern: the
        // declaration a `#[command(...)]` annotates (an enum variant
        // opening a brace block, say) is not always itself an
        // attachable signature line, and in the -U0 diff shape the
        // attribute line is the only changed line there is.
        if let Some(attribute) = parse_command_attribute(line) {
            signatures
                .entry("command".to_string())
                .or_default()
                .insert(attribute);
        }
    }
    // Same re-export transitivity as in
    // `extract_public_surface_declarations`: a restricted item behind
    // a bare `pub use` in the same text contributes its declaration
    // signature under its own name.
    let reexported = reexported_source_names(&cleaned);
    // `const` / `static` signatures are taken from the comments-only
    // intermediate (`route_text`, computed above) instead of the fully
    // blanked line: a constant's literal value is its surface, so
    // blanking it would make a value-only change signature-identical.
    // Recognition still runs on the fully blanked line, so a literal
    // containing `;`, `//`-like text, or escaped quotes cannot break
    // declaration parsing, and comments trailing the declaration stay
    // blanked in both intermediates. All other kinds keep the blanked
    // line, so literals inside fn bodies and other items do not leak
    // into their signatures.
    let signature_line = |kind: &str, line: &str, preserved_line: &str| {
        if kind == "const" || kind == "static" {
            declaration_signature(kind, preserved_line)
        } else {
            declaration_signature(kind, line)
        }
    };
    // Serde attributes awaiting their declaration: a `#[serde(...)]`
    // line contributes only when it directly precedes a declaration
    // or member (stacked attribute lines and blank lines in between
    // keep it pending; any other code line drops it), mirroring how
    // clap attributes sit on the fields/items they annotate.
    let mut pending_serde: Vec<String> = Vec::new();
    // Clap command attributes awaiting their declaration, with the
    // same placement rule as pending_serde: a `#[command(...)]` line
    // contributes its contents to the enclosing declaration's
    // signature only when it directly precedes a declaration or
    // member (stacked attribute lines and blank lines keep it
    // pending; any other code line drops it).
    let mut pending_command: Vec<String> = Vec::new();
    // Clap argument attributes awaiting their field, with the same
    // placement rule: a `#[arg(...)]` line contributes only when it
    // directly precedes the field it annotates. On attachment the
    // attribute is recorded under a key qualified by the owning
    // command and field (`clap-arg:<Owner>.<field>`), because the
    // unconditional shared `clap-arg` key alone cancels a change
    // whenever both spellings occur on other options in the file.
    let mut pending_clap_args: Vec<String> = Vec::new();
    // The enclosing container (struct / enum / trait / union / mod,
    // an `impl` target type, or a TypeScript class / interface /
    // enum / namespace) most recently declared in the text. Member
    // signature keys are qualified with it (`field:<Owner>.<name>`),
    // so moving an unchanged member between two containers in one
    // file changes the map instead of cancelling out under a bare
    // `field:<name>` key — and same-named members of two containers
    // no longer merge into one key's signature set. A member with no
    // preceding container in the text (the common -U0 diff shape,
    // where the header line is unchanged context) keeps its bare
    // key. The declarations name set is deliberately not qualified:
    // the gate compares only this signature map (step 8 of
    // `evaluate_doc_gate`); the name set is not consumed by the gate,
    // so qualifying it would change no enforcement outcome.
    let mut enclosing: Option<String> = None;
    // The KIND of the enclosing container, tracked alongside its name:
    // the bare-member (variant) pattern may only fire when the
    // enclosing container is actually an enum. A bare token inside a
    // struct / trait / union / mod / impl block is a body or literal
    // fragment, never a variant.
    let mut enclosing_kind: Option<String> = None;
    // The struct-variant scope inside the enclosing enum: a variant
    // header (`Daemon {`) whose payload opens unclosed on its line
    // puts the following, deeper lines inside the variant, so their
    // owners are the variant as well as the enum (the daemon
    // `profile` option's owner is `Commands.Daemon`). A line at or
    // shallower than the header ends the variant scope.
    let mut variant_scope: Option<(String, usize)> = None;
    // The indentation of a `fn` header whose body opened on its line
    // and has not closed yet. Lines deeper than the header are fn
    // body lines: their bare tokens are body expressions (`Ok(out)`
    // is the tail expression of a method, not an enum member), so the
    // variant pattern must not fire for them either. A line at or
    // shallower than the header ends the body.
    let mut fn_body_indent: Option<usize> = None;
    let mut test_scope = TestScopeTracker::new();
    for ((line, original_line), preserved_line) in
        cleaned.lines().zip(text.lines()).zip(route_text.lines())
    {
        let indent = line.len() - line.trim_start().len();
        // Test-only scopes extract nothing (see TestScopeTracker):
        // a test fn rename or a test helper's signature change must
        // not register as public surface.
        if test_scope.suppresses(indent, line) {
            continue;
        }
        if let Some(attribute) = parse_serde_attribute(preserved_line) {
            pending_serde.push(attribute);
        }
        if let Some(attribute) = parse_command_attribute(preserved_line) {
            pending_command.push(attribute);
        }
        if let Some(attribute) = parse_clap_arg_attribute(original_line) {
            pending_clap_args.push(attribute);
        }
        // A non-blank line at or shallower than a variant header
        // ends the variant scope.
        if !line.trim().is_empty()
            && variant_scope
                .as_ref()
                .map(|(_, open_indent)| indent <= *open_indent)
                .unwrap_or(false)
        {
            variant_scope = None;
        }
        // Blank lines carry indent 0 but are not body content: they
        // must not end fn-body tracking, or a tail expression after
        // a blank line inside a fn is read as an enum member.
        if !line.trim().is_empty() && fn_body_indent.map(|i| indent <= i).unwrap_or(false) {
            fn_body_indent = None;
        }
        // A closing brace at column 0 ends the enclosing container.
        if !line.starts_with([' ', '\t']) && line.trim_start().starts_with('}') {
            enclosing = None;
            enclosing_kind = None;
            variant_scope = None;
        }
        // The owner in scope at the START of this line: a line that is
        // both a declaration and a member (a `pub fn` method in an
        // impl block) must be qualified with it, not with any scope
        // the declaration branch below installs for a container kind
        // declared on this same line.
        let enclosing_at_line_start = enclosing.clone();
        let enclosing_kind_at_line_start = enclosing_kind.clone();
        let variant_at_line_start = variant_scope.as_ref().map(|(name, _)| name.clone());
        let in_fn_body_at_line_start = fn_body_indent.is_some();
        let mut line_is_fn = false;
        let mut attach_keys: Vec<String> = Vec::new();
        let mut attach_names: Vec<String> = Vec::new();
        if let Some(pairs) = parse_typescript_exports(line, original_line) {
            if let Some((name, _)) = pairs.first() {
                let container_kind = line
                    .split_whitespace()
                    .find(|token| ["class", "interface", "enum", "namespace"].contains(token))
                    .map(|token| token.to_string());
                enclosing = if container_kind.is_some() {
                    Some(name.clone())
                } else {
                    None
                };
                enclosing_kind = container_kind;
            }
            for (name, signature) in pairs {
                signatures
                    .entry(format!("ts:{name}"))
                    .or_default()
                    .insert(signature);
            }
        } else if let Some(pairs) = parse_use_declarations(line) {
            enclosing = None;
            enclosing_kind = None;
            for (name, full_path) in pairs {
                signatures
                    .entry(format!("use:{name}"))
                    .or_default()
                    .insert(full_path);
            }
        } else if let Some((name, kind)) = parse_public_surface_declaration(line) {
            let signature = signature_line(&kind, line, preserved_line);
            if kind == "fn" {
                line_is_fn = true;
            }
            // Only a container declaration replaces the enclosing
            // scope: a non-container declaration (a method, say) must
            // leave it intact, or every member after the first in a
            // block would lose its owner qualification.
            if ["struct", "enum", "trait", "union", "mod"].contains(&kind.as_str()) {
                enclosing = Some(name.clone());
                enclosing_kind = Some(kind.clone());
            }
            attach_keys.push(name.clone());
            attach_names.push(name.clone());
            signatures.entry(name).or_default().insert(signature);
        } else if let Some((name, kind)) = parse_restricted_surface_declaration(line)
            .filter(|(name, _kind)| reexported.contains(name))
        {
            let signature = signature_line(&kind, line, preserved_line);
            if kind == "fn" {
                line_is_fn = true;
            }
            // Same rule as the public-declaration branch above: only
            // a container declaration replaces the enclosing scope.
            if ["struct", "enum", "trait", "union", "mod"].contains(&kind.as_str()) {
                enclosing = Some(name.clone());
                enclosing_kind = Some(kind.clone());
            }
            attach_keys.push(name.clone());
            attach_names.push(name.clone());
            signatures.entry(name).or_default().insert(signature);
        } else if let Some(owner) = parse_impl_enclosing(line) {
            enclosing = Some(owner);
            enclosing_kind = Some("impl".to_string());
        } else if let Some((name, kind)) = parse_any_container_header(line) {
            // A private container (a clap command enum, a test
            // module) is not itself extracted, but it encloses the
            // members that follow, and they must be qualified with
            // it — the declaration branches above only see bare
            // `pub` containers.
            enclosing = Some(name);
            enclosing_kind = Some(kind);
        }
        if let Some((member_name, key, signature)) = parse_member_declaration(line) {
            if key.starts_with("fn:") {
                line_is_fn = true;
            }
            // The variant / bare-member pattern is the one whose key
            // repeats its whole signature (`member:<content>`): it
            // may fire only in an enum context — an enclosing enum,
            // or no enclosing context at all (the common -U0 diff
            // shape, where the enum header is unchanged context and
            // the variant line is all there is). Inside a fn body or
            // a known non-enum container the same shape is a body
            // expression, and recording it made a pure body refactor
            // (`Ok(out)` -> `Ok(embeddings)`) read as a surface
            // change.
            let is_variant_token = key.strip_prefix("member:") == Some(signature.as_str());
            let variant_allowed = !is_variant_token
                || (!in_fn_body_at_line_start
                    && match enclosing_kind_at_line_start.as_deref() {
                        None => true,
                        Some(kind) => kind == "enum",
                    });
            if variant_allowed {
                let key = match &enclosing_at_line_start {
                    Some(owner) => match key.split_once(':') {
                        Some((prefix, rest)) => format!("{prefix}:{owner}.{rest}"),
                        None => key,
                    },
                    None => key,
                };
                attach_keys.push(key.clone());
                attach_names.push(member_name.clone());
                signatures.entry(key).or_default().insert(signature);
            }
        }
        // A multiline struct-variant header opens the variant scope
        // for the fields that follow (a tuple variant's payload is
        // types, not fields, so only `{` opens a scope).
        if enclosing_kind_at_line_start.as_deref() == Some("enum") {
            if let Some((variant_name, _key, _signature)) =
                parse_multiline_variant_header(line)
            {
                let trimmed = line.trim();
                if trimmed[variant_name.len()..].trim_start().starts_with('{') {
                    variant_scope = Some((variant_name, indent));
                }
            }
        }
        // A fn header whose body brace opens unclosed on its line
        // puts the following, deeper lines inside the fn body.
        if line_is_fn
            && line.matches('{').count() > line.matches('}').count()
        {
            fn_body_indent = Some(indent);
        }
        if !attach_keys.is_empty() {
            // The pending serde contents enter the enclosing
            // declaration's signature set (as a `serde(...)` signature)
            // and the shared `serde` key, following the clap pattern:
            // changing any one attribute makes the added and removed
            // signature sets differ.
            for key in &attach_keys {
                for attribute in &pending_serde {
                    signatures
                        .entry(key.clone())
                        .or_default()
                        .insert(format!("serde({attribute})"));
                }
            }
            for attribute in &pending_serde {
                signatures
                    .entry("serde".to_string())
                    .or_default()
                    .insert(attribute.clone());
            }
            pending_serde.clear();
            // The pending command contents likewise enter the
            // enclosing declaration's signature set (as a
            // `command(...)` signature); the shared `command` key
            // was already populated unconditionally above, following
            // the clap-arg pattern.
            for key in &attach_keys {
                for attribute in &pending_command {
                    signatures
                        .entry(key.clone())
                        .or_default()
                        .insert(format!("command({attribute})"));
                }
            }
            pending_command.clear();
            // The pending clap-arg contents enter the annotated
            // field's signature set (as an `arg(...)` signature) and
            // a key qualified by the owning command and field: the
            // shared `clap-arg` key (populated unconditionally in the
            // schema scan above) cannot distinguish two options whose
            // spellings both occur elsewhere in the file.
            for key in &attach_keys {
                for attribute in &pending_clap_args {
                    signatures
                        .entry(key.clone())
                        .or_default()
                        .insert(format!("arg({attribute})"));
                }
            }
            if !pending_clap_args.is_empty() {
                let owner_path = match (&enclosing_at_line_start, &variant_at_line_start) {
                    (Some(owner), Some(variant)) => Some(format!("{owner}.{variant}")),
                    (Some(owner), None) => Some(owner.clone()),
                    (None, Some(variant)) => Some(variant.clone()),
                    (None, None) => None,
                };
                if let Some(owner_path) = owner_path {
                    for name in &attach_names {
                        for attribute in &pending_clap_args {
                            signatures
                                .entry(format!("clap-arg:{owner_path}.{name}"))
                                .or_default()
                                .insert(attribute.clone());
                        }
                    }
                }
            }
            pending_clap_args.clear();
        } else {
            let preserved_trimmed = preserved_line.trim();
            if !preserved_trimmed.is_empty() && !preserved_trimmed.starts_with("#[") {
                pending_serde.clear();
                pending_command.clear();
                pending_clap_args.clear();
            }
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
        if inspected && (tracked || !added.is_empty() || !removed.is_empty()) {
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

/// The (baseline, current) full texts of a watched path for the
/// surface-diff check: the file's content at the committed baseline
/// (the start of the shared committed range — `@{u}` or
/// `origin/main` — or `HEAD` when no committed range resolves) and
/// its current working-tree content. Comparing the surfaces of
/// these two texts yields the NET surface change across committed
/// and uncommitted edits combined. Unioning the successive diffs'
/// lines instead puts a declaration added by a commit on both
/// sides whenever a later uncommitted edit re-touches its line, so
/// a newly added API whose only uncommitted change is a same-line
/// body edit cancelled out and escaped the gate. A path absent at
/// the baseline reads as empty there; a path deleted from the
/// working tree reads as empty here.
fn watched_surface_texts(root: &Path, rel_path: &str) -> (String, String) {
    let current = std::fs::read_to_string(root.join(rel_path)).unwrap_or_default();
    if !root.join(".git").exists() {
        return (String::new(), current);
    }
    let range = crate::workflow::connector::committed_diff_range(root);
    let baseline_ref = range
        .as_deref()
        .and_then(|range| range.split("..").next())
        .unwrap_or("HEAD");
    let baseline_path = range
        .as_deref()
        .and_then(|range| rename_source(root, range, rel_path))
        .or_else(|| rename_source(root, "HEAD", rel_path))
        .unwrap_or_else(|| rel_path.to_string());
    let baseline = std::process::Command::new("git")
        .args(["show", &format!("{baseline_ref}:{baseline_path}")])
        .current_dir(root)
        .output()
        .ok()
        .filter(|out| out.status.success())
        .map(|out| String::from_utf8_lossy(&out.stdout).into_owned())
        .unwrap_or_default();
    (baseline, current)
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
    //    the rule's merged before/after signature maps differ — a public
    //    declaration or SS7 identifier was added, removed, or changed
    //    signature in one of the rule's changed watched paths. The maps
    //    are extracted from the baseline tree's file text and the
    //    current working file text (see `watched_surface_texts`), so
    //    the comparison is the NET surface change across committed and
    //    uncommitted edits: a body-only edit yields identical
    //    signatures on both sides and does not fire, while an API
    //    added by a commit stays visible on the after side even when
    //    an uncommitted edit re-touches its declaration line.
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
            let (before_text, after_text) = watched_surface_texts(root, cp);
            for (key, sigs) in extract_public_surface_signatures(&after_text) {
                added_signatures.entry(key).or_default().extend(sigs);
            }
            for (key, sigs) in extract_public_surface_signatures(&before_text) {
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
        // `pub(crate)` is restricted visibility, not public surface.
        assert!(!decls.contains("ScopedEnum"), "{decls:?}");
    }

    #[test]
    fn extract_public_surface_declarations_parses_grouped_use() {
        let decls = extract_public_surface_declarations("pub use a::{Alpha, Beta};");
        let expected: std::collections::BTreeSet<String> = ["Alpha", "Beta"]
            .iter()
            .map(|name| name.to_string())
            .collect();
        assert_eq!(decls, expected);
    }

    #[test]
    fn extract_public_surface_declarations_parses_grouped_use_with_aliases_and_nesting() {
        let decls = extract_public_surface_declarations(
            "pub use a::{b::{C, D}, E as RenamedE, self};",
        );
        let expected: std::collections::BTreeSet<String> = ["C", "D", "RenamedE", "a"]
            .iter()
            .map(|name| name.to_string())
            .collect();
        assert_eq!(decls, expected);
    }

    #[test]
    fn extract_public_surface_declarations_parses_glob_use() {
        let decls = extract_public_surface_declarations("pub use a::b::*;");
        let expected: std::collections::BTreeSet<String> =
            ["b"].iter().map(|name| name.to_string()).collect();
        assert_eq!(decls, expected);
        let signatures = extract_public_surface_signatures("pub use a::b::*;");
        let expected_signature: std::collections::BTreeSet<String> =
            ["a::b::*"].iter().map(|sig| sig.to_string()).collect();
        assert_eq!(
            signatures.get("use:b"),
            Some(&expected_signature),
            "{signatures:?}"
        );
    }

    #[test]
    fn extract_public_surface_declarations_use_member_move_changes_signature() {
        let before_decls = extract_public_surface_declarations("pub use a::{Alpha};");
        let after_decls = extract_public_surface_declarations("pub use b::{Alpha};");
        assert_eq!(before_decls, after_decls);
        let before = extract_public_surface_signatures("pub use a::{Alpha};");
        let after = extract_public_surface_signatures("pub use b::{Alpha};");
        assert_ne!(before.get("use:Alpha"), after.get("use:Alpha"));
    }

    #[test]
    fn extract_public_surface_declarations_single_use_unchanged() {
        let decls = extract_public_surface_declarations("pub use crate::plugins::Alpha;");
        let expected: std::collections::BTreeSet<String> =
            ["Alpha"].iter().map(|name| name.to_string()).collect();
        assert_eq!(decls, expected);
        let renamed =
            extract_public_surface_declarations("pub use crate::store::Widget as RenamedWidget;");
        let expected_renamed: std::collections::BTreeSet<String> = ["RenamedWidget"]
            .iter()
            .map(|name| name.to_string())
            .collect();
        assert_eq!(renamed, expected_renamed);
    }

    #[test]
    fn extract_typescript_exports_recognizes_declaration_kinds() {
        let text = "\
export const DEFAULT_ENDPOINT = 'http://127.0.0.1:34343/mcp'
export function resolveConfig(
export interface PluginConfig {
export type ResolvedConfig = PluginConfig
export class NeuroStrataMCPClient {
export async function postTurnCandidates(
";
        let decls = extract_public_surface_declarations(text);
        for expected in [
            "DEFAULT_ENDPOINT",
            "resolveConfig",
            "PluginConfig",
            "ResolvedConfig",
            "NeuroStrataMCPClient",
            "postTurnCandidates",
        ] {
            assert!(decls.contains(expected), "missing {expected}: {decls:?}");
        }
    }

    #[test]
    fn extract_typescript_exports_recognizes_reexports() {
        let decls = extract_public_surface_declarations(
            "export { Alpha, Beta as Gamma, type Delta } from \"./x\";",
        );
        for expected in ["Alpha", "Gamma", "Delta"] {
            assert!(decls.contains(expected), "missing {expected}: {decls:?}");
        }
        assert!(!decls.contains("Beta"), "{decls:?}");
        let star_as =
            extract_public_surface_declarations("export * as ns from \"./x\";");
        assert!(star_as.contains("ns"), "{star_as:?}");
        let star = extract_public_surface_declarations("export * from \"./module\";");
        assert!(star.contains("module"), "{star:?}");
    }

    #[test]
    fn extract_typescript_exports_ignores_non_exports() {
        let text = "\
const LOCAL = 1
function helper() {}
// export function fake() {}
";
        let decls = extract_public_surface_declarations(text);
        assert!(decls.is_empty(), "{decls:?}");
    }

    #[test]
    fn extract_typescript_export_signatures_distinguish_body_and_value() {
        let before = extract_public_surface_signatures("export function f(): number { return 1; }");
        let after = extract_public_surface_signatures("export function f(): number { return 2; }");
        assert_eq!(
            before.get("ts:f"),
            after.get("ts:f"),
            "body-only function edit must be signature-identical: {before:?} vs {after:?}"
        );
        assert!(before.contains_key("ts:f"), "{before:?}");
        let before_const =
            extract_public_surface_signatures("export const DEFAULT_ENDPOINT = 'a'");
        let after_const =
            extract_public_surface_signatures("export const DEFAULT_ENDPOINT = 'b'");
        assert_ne!(
            before_const.get("ts:DEFAULT_ENDPOINT"),
            after_const.get("ts:DEFAULT_ENDPOINT"),
            "exported const value change must differ: {before_const:?} vs {after_const:?}"
        );
    }

    fn setup_git_typescript_repo(root: &Path, initial_source: &str) {
        let docs_dir = root.join("docs");
        std::fs::create_dir_all(docs_dir.join("vision")).unwrap();
        std::fs::create_dir_all(docs_dir.join("features-api")).unwrap();
        std::fs::create_dir_all(docs_dir.join("extend")).unwrap();
        std::fs::create_dir_all(root.join("plugins").join("core")).unwrap();
        std::fs::write(
            docs_dir.join("vision").join("target_doc.md"),
            "## Introduction\nThe target_doc surface. TargetTrait\n",
        )
        .unwrap();
        std::fs::write(
            root.join("plugins").join("core").join("index.ts"),
            initial_source,
        )
        .unwrap();
        write_manifest(
            &docs_dir,
            r###"[{
                "id": "test-rule-1",
                "tier": "vision",
                "watch_paths": ["plugins/core/index.ts"],
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

    #[test]
    fn typescript_export_change_blocks_without_doc_update() {
        let tmp_dir = tempfile::tempdir().unwrap();
        let root = tmp_dir.path();
        setup_git_typescript_repo(
            root,
            "export function buildHeaders(token: string) {}\n",
        );
        std::fs::write(
            root.join("plugins").join("core").join("index.ts"),
            "export function buildRequestHeaders(token: string) {}\nexport interface NewSurface {}\n",
        )
        .unwrap();
        commit_all(root, "typescript export change");

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
    fn typescript_internal_change_passes_without_doc_update() {
        let tmp_dir = tempfile::tempdir().unwrap();
        let root = tmp_dir.path();
        setup_git_typescript_repo(
            root,
            "export function buildHeaders(token: string) {}\nfunction helper() { return 1; }\n",
        );
        std::fs::write(
            root.join("plugins").join("core").join("index.ts"),
            "export function buildHeaders(token: string) {}\nfunction helper() { return 2; }\n",
        )
        .unwrap();
        commit_all(root, "typescript internal change");

        let report = evaluate_doc_gate(root, None);
        assert!(
            report.passed,
            "internal-only TS change produced violations: {:?}",
            report.violations
        );
    }

    #[test]
    fn extract_public_surface_declarations_ignores_block_comment_contents() {
        let text = "\
/*
pub struct CommentExample {}
*/
pub struct RealExample {}
";
        let decls = extract_public_surface_declarations(text);
        assert!(!decls.contains("CommentExample"), "{decls:?}");
        assert!(decls.contains("RealExample"), "{decls:?}");
    }

    #[test]
    fn extract_public_surface_declarations_ignores_multiline_string_contents() {
        let text = "let s = \"\npub struct StringExample {}\n\";\npub struct RealExample {}\n";
        let decls = extract_public_surface_declarations(text);
        assert!(!decls.contains("StringExample"), "{decls:?}");
        assert!(decls.contains("RealExample"), "{decls:?}");
    }

    #[test]
    fn extract_public_surface_declarations_ignores_line_comment_and_doc_comment_contents() {
        let text = "// pub struct LineExample {}\n/// pub struct DocExample {}\n";
        let decls = extract_public_surface_declarations(text);
        assert!(decls.is_empty(), "{decls:?}");
    }

    #[test]
    fn extract_public_surface_declarations_ignores_member_lines_in_block_comment() {
        let text = "/*\n    fn fetch(&self, id: u64);\n*/\n";
        let decls = extract_public_surface_declarations(text);
        assert!(!decls.contains("fetch"), "{decls:?}");
    }

    #[test]
    fn blank_comments_and_strings_preserves_code_around_strings_with_comment_markers() {
        let decls =
            extract_public_surface_declarations("pub const URL: &str = \"http://x/*y*/\";");
        assert!(decls.contains("URL"), "{decls:?}");
    }

    #[test]
    fn blank_comments_and_strings_handles_nested_block_comments_and_raw_strings() {
        let text = "\
/* outer
/* inner */
pub struct Hidden {}
*/
let doc = r#\"
pub struct RawHidden {}
\"#;
pub struct AfterAll {}
";
        let decls = extract_public_surface_declarations(text);
        assert!(!decls.contains("Hidden"), "{decls:?}");
        assert!(!decls.contains("RawHidden"), "{decls:?}");
        assert!(decls.contains("AfterAll"), "{decls:?}");
    }

    #[test]
    fn blank_comments_and_strings_lifetime_is_not_a_string_start() {
        let text = "pub fn get(&'a self) -> &'a str { \"\" }";
        let decls = extract_public_surface_declarations(text);
        assert!(decls.contains("get"), "{decls:?}");
        let signatures = extract_public_surface_signatures(text);
        let expected_signature: std::collections::BTreeSet<String> =
            ["pub fn get(&'a self) -> &'a str"]
                .iter()
                .map(|sig| sig.to_string())
                .collect();
        assert_eq!(
            signatures.get("get"),
            Some(&expected_signature),
            "{signatures:?}"
        );
    }

    #[test]
    fn extract_public_surface_declarations_ss7_identifiers_survive_blanking() {
        let text = "\"name\": \"neurostrata_some_tool\"";
        let decls = extract_public_surface_declarations(text);
        assert!(decls.contains("neurostrata_some_tool"), "{decls:?}");
    }

    #[test]
    fn blank_comments_only_preserves_string_contents() {
        // The intermediate reserved for the pre-blanking extraction
        // point: comments blanked, string literal contents preserved.
        let text = "/* pub struct Hidden {} */\nlet path = \"/health\"; // route\n";
        let cleaned = blank_comments_only(text);
        assert!(!cleaned.contains("Hidden"), "{cleaned:?}");
        assert!(cleaned.contains("\"/health\""), "{cleaned:?}");
        assert!(!cleaned.contains("route"), "{cleaned:?}");
    }

    #[test]
    fn extract_public_surface_declarations_recognizes_http_routes() {
        let text = "        .route(\"/health\", get(|| async { \"ok\" }))\n        .route(\"/ingest\", post(handle_ingest))\n        .route(\"/memory/archive\", post(handle_memory_archive))\n";
        let decls = extract_public_surface_declarations(text);
        assert!(decls.contains("route:/health"), "{decls:?}");
        assert!(decls.contains("route:/ingest"), "{decls:?}");
        assert!(decls.contains("route:/memory/archive"), "{decls:?}");
        let sigs = extract_public_surface_signatures(text);
        assert!(
            sigs.get("route:/health")
                .is_some_and(|s| s.contains("route:/health:get")),
            "{sigs:?}"
        );
        assert!(
            sigs.get("route:/ingest")
                .is_some_and(|s| s.contains("route:/ingest:post")),
            "{sigs:?}"
        );
        assert!(
            sigs.get("route:/memory/archive")
                .is_some_and(|s| s.contains("route:/memory/archive:post")),
            "{sigs:?}"
        );
    }

    #[test]
    fn extract_public_surface_declarations_recognizes_split_route_path() {
        let text = "        .route(\n            \"/storage/status\",\n            get(handle_http_storage_status).post(handle_http_storage_status),\n        )\n";
        let decls = extract_public_surface_declarations(text);
        assert!(decls.contains("route:/storage/status"), "{decls:?}");
        let sigs = extract_public_surface_signatures(text);
        assert!(
            sigs.get("route:/storage/status")
                .is_some_and(|s| s.contains("route:/storage/status:-")),
            "{sigs:?}"
        );
    }

    #[test]
    fn extract_public_surface_declarations_route_method_change_differs() {
        let before = extract_public_surface_signatures("        .route(\"/health\", get(h))\n");
        let after = extract_public_surface_signatures("        .route(\"/health\", post(h))\n");
        assert!(before.contains_key("route:/health"), "{before:?}");
        assert!(after.contains_key("route:/health"), "{after:?}");
        assert_ne!(before.get("route:/health"), after.get("route:/health"));
    }

    #[test]
    fn extract_public_surface_declarations_route_handler_swap_is_identical() {
        let before_text = "        .route(\"/health\", get(handle_a))\n";
        let after_text = "        .route(\"/health\", get(handle_b))\n";
        let before = extract_public_surface_signatures(before_text);
        let after = extract_public_surface_signatures(after_text);
        assert_eq!(before.get("route:/health"), after.get("route:/health"));
        let before_decls = extract_public_surface_declarations(before_text);
        let after_decls = extract_public_surface_declarations(after_text);
        assert_eq!(before_decls, after_decls);
    }

    #[test]
    fn extract_public_surface_declarations_ignores_commented_route() {
        let text = "/*\n        .route(\"/ghost\", get(h))\n*/\n";
        let decls = extract_public_surface_declarations(text);
        assert!(!decls.contains("route:/ghost"), "{decls:?}");
        let sigs = extract_public_surface_signatures(text);
        assert!(!sigs.contains_key("route:/ghost"), "{sigs:?}");
    }

    #[test]
    fn extract_public_surface_declarations_ignores_plain_path_strings() {
        let text = "let p = \"/health\";\n    let q = \"/health\";\n";
        let decls = extract_public_surface_declarations(text);
        assert!(!decls.contains("route:/health"), "{decls:?}");
        let sigs = extract_public_surface_signatures(text);
        assert!(!sigs.contains_key("route:/health"), "{sigs:?}");
    }

    fn setup_git_daemon_repo(root: &Path, initial_source: &str) {
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
        std::fs::write(root.join("src").join("daemon.rs"), initial_source).unwrap();
        write_manifest(
            &docs_dir,
            r###"[{
                "id": "test-rule-1",
                "tier": "vision",
                "watch_paths": ["src/daemon.rs"],
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

    #[test]
    fn http_route_addition_blocks_without_doc_update() {
        let tmp_dir = tempfile::tempdir().unwrap();
        let root = tmp_dir.path();
        setup_git_daemon_repo(
            root,
            "        .route(\"/health\", get(handle_health))\n",
        );
        std::fs::write(
            root.join("src").join("daemon.rs"),
            "        .route(\"/health\", get(handle_health))\n        .route(\"/new-endpoint\", post(handle_new))\n",
        )
        .unwrap();
        commit_all(root, "route addition");

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
    fn http_handler_only_change_passes_without_doc_update() {
        let tmp_dir = tempfile::tempdir().unwrap();
        let root = tmp_dir.path();
        setup_git_daemon_repo(
            root,
            "        .route(\"/health\", get(handle_a))\n",
        );
        std::fs::write(
            root.join("src").join("daemon.rs"),
            "        .route(\"/health\", get(handle_b))\n",
        )
        .unwrap();
        commit_all(root, "handler only change");

        let report = evaluate_doc_gate(root, None);
        assert!(
            report.passed,
            "handler-only change produced violations: {:?}",
            report.violations
        );
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
    fn extract_public_surface_signatures_const_string_value_change_differs() {
        let before = extract_public_surface_signatures("pub const GREETING: &str = \"a\";");
        let after = extract_public_surface_signatures("pub const GREETING: &str = \"b\";");
        assert_ne!(before, after, "before={before:?} after={after:?}");
        assert_ne!(before.get("GREETING"), after.get("GREETING"));
    }

    #[test]
    fn extract_public_surface_signatures_static_string_value_change_differs() {
        let before =
            extract_public_surface_signatures("pub static SCHEMA_PATH: &str = \"a.json\";");
        let after = extract_public_surface_signatures("pub static SCHEMA_PATH: &str = \"b.json\";");
        assert_ne!(before.get("SCHEMA_PATH"), after.get("SCHEMA_PATH"));
    }

    #[test]
    fn extract_public_surface_signatures_const_char_and_raw_string_changes_differ() {
        let before = extract_public_surface_signatures("pub const SEPARATOR: char = 'a';");
        let after = extract_public_surface_signatures("pub const SEPARATOR: char = 'b';");
        assert_ne!(before.get("SEPARATOR"), after.get("SEPARATOR"));
        let before = extract_public_surface_signatures("pub const TEMPLATE: &str = r#\"a\"#;");
        let after = extract_public_surface_signatures("pub const TEMPLATE: &str = r#\"b\"#;");
        assert_ne!(before.get("TEMPLATE"), after.get("TEMPLATE"));
    }

    #[test]
    fn extract_public_surface_signatures_const_literal_edge_contents_differ() {
        // Escaped quotes, a `;` inside the literal, and `//`-like
        // content must all survive signature construction without
        // breaking recognition of the declaration itself.
        for (before_line, after_line) in [
            (
                "pub const QUOTED: &str = \"a\\\"b\";",
                "pub const QUOTED: &str = \"a\\\"c\";",
            ),
            (
                "pub const SEMI: &str = \"a;b\";",
                "pub const SEMI: &str = \"a;c\";",
            ),
            (
                "pub const URL: &str = \"http://x/a\";",
                "pub const URL: &str = \"http://x/b\";",
            ),
        ] {
            let before = extract_public_surface_signatures(before_line);
            let after = extract_public_surface_signatures(after_line);
            assert_ne!(before, after, "before={before:?} after={after:?}");
            assert_eq!(before.len(), 1, "{before:?}");
        }
    }

    #[test]
    fn extract_public_surface_signatures_const_comment_change_is_identical() {
        let before = extract_public_surface_signatures("pub const GREETING: &str = \"a\"; // one");
        let after = extract_public_surface_signatures("pub const GREETING: &str = \"a\"; // two");
        assert_eq!(before, after);
    }

    #[test]
    fn extract_public_surface_signatures_fn_body_string_change_is_identical() {
        // Literal preservation is scoped to const/static declarations:
        // a string change inside a fn body is not signature surface.
        let before = extract_public_surface_signatures("pub fn label() -> &'static str { \"a\" }");
        let after = extract_public_surface_signatures("pub fn label() -> &'static str { \"b\" }");
        assert_eq!(before, after);
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

    #[test]
    fn extract_public_surface_declarations_recognizes_trait_method_signature_lines() {
        let before = extract_public_surface_declarations("    fn fetch(&self, id: u64);");
        let after = extract_public_surface_declarations("    fn fetch(&self, id: String);");
        assert!(before.contains("fetch"), "{before:?}");
        assert!(after.contains("fetch"), "{after:?}");
        let before_sigs = extract_public_surface_signatures("    fn fetch(&self, id: u64);");
        let after_sigs = extract_public_surface_signatures("    fn fetch(&self, id: String);");
        assert_ne!(before_sigs.get("fn:fetch"), after_sigs.get("fn:fetch"));
    }

    #[test]
    fn extract_public_surface_declarations_recognizes_pub_field_type_lines() {
        let decls = extract_public_surface_declarations("    pub id: u64,");
        assert!(decls.contains("id"), "{decls:?}");
    }

    #[test]
    fn extract_public_surface_declarations_recognizes_enum_variant_lines() {
        let decls = extract_public_surface_declarations("    Variant(u64),\n    PlainVariant,");
        assert!(decls.contains("Variant"), "{decls:?}");
        assert!(decls.contains("PlainVariant"), "{decls:?}");
    }

    #[test]
    fn body_expression_in_trait_method_is_not_an_enum_member() {
        // W1 probe (Codex PR #162): the tail expression of a trait
        // default method (`Ok(out)` in Embedder::embed_batch) is a
        // body expression, not an enum variant: a body-only refactor
        // renaming the returned variable must be signature-identical.
        let before = extract_public_surface_signatures(
            "pub trait Embedder {\n    async fn embed_batch(&self) -> Result<Vec<Vec<f32>>> {\n        Ok(out)\n    }\n}",
        );
        let after = extract_public_surface_signatures(
            "pub trait Embedder {\n    async fn embed_batch(&self) -> Result<Vec<Vec<f32>>> {\n        Ok(embeddings)\n    }\n}",
        );
        assert_eq!(before, after, "before={before:?} after={after:?}");
        assert!(
            !before.keys().any(|k| k.contains("Ok")),
            "body expression extracted as a member: {before:?}"
        );
    }

    #[test]
    fn body_expression_in_impl_method_is_not_an_enum_member() {
        let before = extract_public_surface_signatures(
            "impl Foo {\n    fn build(&self) -> Result<Foo> {\n        Ok(foo)\n    }\n}",
        );
        let after = extract_public_surface_signatures(
            "impl Foo {\n    fn build(&self) -> Result<Foo> {\n        Ok(bar)\n    }\n}",
        );
        assert_eq!(before, after, "before={before:?} after={after:?}");
    }

    #[test]
    fn body_expression_in_top_level_fn_is_not_an_enum_member() {
        let before =
            extract_public_surface_signatures("pub fn build() -> Result<Foo> {\n    Ok(out)\n}");
        let after = extract_public_surface_signatures(
            "pub fn build() -> Result<Foo> {\n    Ok(embeddings)\n}",
        );
        assert_eq!(before, after, "before={before:?} after={after:?}");
    }

    #[test]
    fn body_expression_after_blank_line_in_top_level_fn_is_not_an_enum_member() {
        // Wave-3 review probe: a blank line inside a free fn has
        // indent 0, which must not end fn-body tracking — otherwise
        // the tail expression after the blank line is extracted as
        // an enum member (the W1 false-positive class).
        let before = extract_public_surface_signatures(
            "pub fn f() -> Result<Foo> {\n\n    Ok(Path::UNIT)\n}",
        );
        let after = extract_public_surface_signatures(
            "pub fn f() -> Result<Foo> {\n\n    Ok(Path::OTHER)\n}",
        );
        assert_eq!(before, after, "before={before:?} after={after:?}");
        assert!(
            !before.keys().any(|k| k.contains("Ok")),
            "body expression extracted as a member: {before:?}"
        );
    }

    #[test]
    fn blank_line_inside_enum_does_not_disturb_variant_extraction() {
        let with_blank =
            extract_public_surface_signatures("pub enum E {\n    Ok(u64),\n\n    Plain,\n}");
        let without_blank =
            extract_public_surface_signatures("pub enum E {\n    Ok(u64),\n    Plain,\n}");
        assert_eq!(with_blank, without_blank, "with_blank={with_blank:?}");
        let changed =
            extract_public_surface_signatures("pub enum E {\n    Ok(u64),\n\n    Other,\n}");
        assert_ne!(with_blank, changed, "with_blank={with_blank:?}");
    }

    #[test]
    fn variant_shaped_line_after_fn_closing_brace_still_registers() {
        // Fn-body tracking ends at the fn's closing brace: a bare
        // member-shaped line after the fn, at body indent, is
        // extracted exactly as it was before body tracking existed
        // (member lines are only recognized when indented, so the
        // post-fn line sits at the body's indent level).
        let before = extract_public_surface_signatures(
            "pub fn f() -> Result<Foo> {\n\n    Ok(Path::UNIT)\n}\n    Bare(u64),",
        );
        let after = extract_public_surface_signatures(
            "pub fn f() -> Result<Foo> {\n\n    Ok(Path::UNIT)\n}\n    Bare(String),",
        );
        assert_ne!(before, after, "before={before:?} after={after:?}");
        assert!(
            before.keys().any(|k| k.contains("Bare")),
            "post-fn member line not extracted: {before:?}"
        );
    }

    #[test]
    fn enum_variant_with_type_payload_still_differs() {
        // Control: real enum variants keep registering, including a
        // variant whose name collides with a body-expression callee.
        let before = extract_public_surface_signatures("pub enum E {\n    Ok(u64),\n    Plain,\n}");
        let after =
            extract_public_surface_signatures("pub enum E {\n    Ok(String),\n    Plain,\n}");
        assert_ne!(before, after, "before={before:?} after={after:?}");
        let after2 = extract_public_surface_signatures("pub enum E {\n    Ok(u64),\n    Other,\n}");
        assert_ne!(before, after2, "before={before:?} after={after2:?}");
    }

    #[test]
    fn extract_public_surface_declarations_recognizes_multiline_signature_continuation() {
        let decls = extract_public_surface_declarations("        text: String,");
        assert!(decls.contains("text"), "{decls:?}");
        let boundary = extract_public_surface_declarations("pub fn embed(");
        assert!(boundary.contains("embed"), "{boundary:?}");
    }

    #[test]
    fn extract_public_surface_declarations_recognizes_extern_c_fn() {
        let decls = extract_public_surface_declarations(
            "pub extern \"C\" fn exported_op() {}\npub unsafe extern \"C\" fn raw_exported_op() {}",
        );
        assert!(decls.contains("exported_op"), "{decls:?}");
        assert!(decls.contains("raw_exported_op"), "{decls:?}");
    }

    #[test]
    fn extract_public_surface_declarations_ignores_column_zero_private_fn() {
        let decls = extract_public_surface_declarations("fn helper() -> i32 { 1 }");
        assert!(decls.is_empty(), "{decls:?}");
    }

    #[test]
    fn extract_public_surface_declarations_ignores_struct_literal_member_lines() {
        let decls = extract_public_surface_declarations("        id: 1,");
        assert!(decls.is_empty(), "{decls:?}");
    }

    #[test]
    fn extract_public_surface_declarations_ignores_live_struct_literal_reference_lines() {
        // The verbatim added lines from the live src/rdf/action.rs diff:
        // a struct literal in test code. None of them is a member line;
        // `domains: &[],` is a reference value, never a type.
        let decls = extract_public_surface_declarations(
            "                namespace: \"ns-batch-idx\",\n                query: \"zephyrquartz99\",\n                limit: 10,\n                include_tasks: false,\n                include_archived: false,\n                domains: &[],\n                principal: \"alice\",\n                fuzzy_max_distance: 0,",
        );
        assert!(decls.is_empty(), "{decls:?}");
    }

    #[test]
    fn extract_public_surface_declarations_ignores_reference_value_member_lines() {
        for line in [
            "        domains: &[],",
            "        values: &[1, 2],",
            "        field: &some_var,",
            "        field: &mut some_var,",
            "        field: &(some_var),",
        ] {
            let decls = extract_public_surface_declarations(line);
            assert!(decls.is_empty(), "{line} yielded {decls:?}");
        }
    }

    #[test]
    fn extract_public_surface_declarations_recognizes_reference_type_continuation_lines() {
        for (line, name) in [
            ("        text: &str,", "text"),
            ("        target: &String,", "target"),
            ("        label: &'static str,", "label"),
            ("        handler: &'a mut String,", "handler"),
            ("        callback: &dyn Fn(u8),", "callback"),
            ("        nested: &&str,", "nested"),
        ] {
            let decls = extract_public_surface_declarations(line);
            assert!(decls.contains(name), "{line} yielded {decls:?}");
        }
    }

    #[test]
    fn extract_public_surface_declarations_recognizes_slice_type_continuation_lines() {
        for (line, name) in [
            ("        bytes: &[u8],", "bytes"),
            ("        names: &[String],", "names"),
            ("        words: &[&str],", "words"),
            ("        buffer: &mut [u8],", "buffer"),
            ("        fixed: &[u8; 32],", "fixed"),
        ] {
            let decls = extract_public_surface_declarations(line);
            assert!(decls.contains(name), "{line} yielded {decls:?}");
        }
    }

    #[test]
    fn extract_public_surface_declarations_ignores_keyword_bare_tokens() {
        // Every spelling in the pattern-4 keyword set (strict + reserved
        // 2021, plus `gen`): a bare keyword can never name a variant or
        // member, so none of these lines extracts.
        for keyword in [
            "as", "async", "await", "break", "const", "continue", "crate", "dyn", "else", "enum",
            "extern", "false", "fn", "for", "if", "impl", "in", "let", "loop", "match", "mod",
            "move", "mut", "pub", "ref", "return", "self", "Self", "static", "struct", "super",
            "trait", "true", "try", "type", "unsafe", "use", "where", "while", "abstract",
            "become", "box", "do", "final", "macro", "override", "priv", "typeof", "unsized",
            "virtual", "yield", "gen",
        ] {
            let line = format!("        {keyword},");
            let decls = extract_public_surface_declarations(&line);
            assert!(decls.is_empty(), "{line} yielded {decls:?}");
        }
        // Named pins, including the no-comma form.
        for line in [
            "        false,",
            "        true,",
            "        true",
            "        Self,",
            "        self,",
            "        crate,",
            "        break,",
            "        continue,",
            "        mut,",
            "        ref,",
        ] {
            let decls = extract_public_surface_declarations(line);
            assert!(decls.is_empty(), "{line} yielded {decls:?}");
        }
        // Control: the raw spelling is a distinct, legal identifier.
        let decls = extract_public_surface_declarations("        r#Self,");
        assert!(decls.contains("r#Self"), "{decls:?}");
    }

    #[test]
    fn extract_public_surface_declarations_ignores_lowercase_bare_tokens() {
        // The Q2 class: bare lowercase tokens are body fragments
        // (positional arguments, shorthand fields, chain receivers,
        // grouped-use members), not variants.
        for line in [
            "        statuses,",
            "        my_var,",
            "        domains,",
            "        namespace",
            "        is_task(r),",
            "        foo::bar(x),",
            "        foo::bar,",
            "        some_function,",
            "        self,",
        ] {
            let decls = extract_public_surface_declarations(line);
            assert!(decls.is_empty(), "{line} yielded {decls:?}");
        }
    }

    #[test]
    fn extract_public_surface_declarations_recognizes_uppercase_variant_lines_after_revision2() {
        for (line, name) in [
            ("        PlainVariant,", "PlainVariant"),
            ("        Variant(u64),", "Variant"),
            ("        Pid(u32),", "Pid"),
            ("        Running { since: String },", "Running"),
            ("        MarkerOrder::First,", "First"),
            ("        Alpha,", "Alpha"),
        ] {
            let decls = extract_public_surface_declarations(line);
            assert!(decls.contains(name), "{line} yielded {decls:?}");
        }
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
    fn body_only_tail_expression_change_passes_without_doc_update() {
        // W1 gate-level probe: a committed body-only refactor of a
        // trait default method's tail expression (`Ok(out)` ->
        // `Ok(embeddings)`) is not a surface change. The -U0 diff
        // carries only the two tail lines, with no enclosing context.
        let tmp_dir = tempfile::tempdir().unwrap();
        let root = tmp_dir.path();
        setup_git_surface_repo(
            root,
            "pub trait TargetTrait {\n    fn finish(&self) -> Result<i32> {\n        Ok(out)\n    }\n}\n",
        );
        std::fs::write(
            root.join("src").join("server.rs"),
            "pub trait TargetTrait {\n    fn finish(&self) -> Result<i32> {\n        Ok(embeddings)\n    }\n}\n",
        )
        .unwrap();
        commit_all(root, "body-only tail expression change");

        let report = evaluate_doc_gate(root, None);
        assert!(
            report.passed,
            "body-only tail-expression change produced violations: {:?}",
            report.violations
        );
    }

    #[test]
    fn committed_new_api_with_uncommitted_body_edit_blocks_without_doc_update() {
        // W2 probe (Codex PR #162): a commit adds a public fn, then
        // an UNCOMMITTED edit changes only its same-line body. The
        // union of both diffs puts the new fn on both sides with an
        // identical (body-stripped) signature, so the new API
        // escaped enforcement.
        let tmp_dir = tempfile::tempdir().unwrap();
        let root = tmp_dir.path();
        setup_git_surface_repo(root, "pub trait TargetTrait {}\n");
        std::fs::write(
            root.join("src").join("server.rs"),
            "pub trait TargetTrait {}\npub fn new_api() { let x = 1; }\n",
        )
        .unwrap();
        commit_all(root, "add new_api");
        // Uncommitted body-only edit to the just-added fn.
        std::fs::write(
            root.join("src").join("server.rs"),
            "pub trait TargetTrait {}\npub fn new_api() { let x = 2; }\n",
        )
        .unwrap();

        let report = evaluate_doc_gate(root, None);
        assert!(!report.passed, "new API escaped the gate: {report:?}");
        assert!(
            report.violations.iter().any(|v| {
                v.kind == "surface_changed_without_doc_update" && v.rule_id == "test-rule-1"
            }),
            "expected co-change violation, got {:?}",
            report.violations
        );
    }

    #[test]
    fn committed_and_uncommitted_body_edits_to_existing_fn_pass() {
        // W2 control: the net surface of a pre-existing fn is
        // unchanged by a committed body edit plus an uncommitted
        // body edit, so the gate must still pass.
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
        commit_all(root, "body edit");
        std::fs::write(
            root.join("src").join("server.rs"),
            "pub trait TargetTrait {}\npub fn compute() -> i32 { 3 }\n",
        )
        .unwrap();

        let report = evaluate_doc_gate(root, None);
        assert!(
            report.passed,
            "net-unchanged surface produced violations: {:?}",
            report.violations
        );
    }

    #[test]
    fn uncommitted_only_new_api_blocks_without_doc_update() {
        // W2 control: a purely uncommitted new API (no committed
        // range involvement) must keep blocking.
        let tmp_dir = tempfile::tempdir().unwrap();
        let root = tmp_dir.path();
        setup_git_surface_repo(root, "pub trait TargetTrait {}\n");
        std::fs::write(
            root.join("src").join("server.rs"),
            "pub trait TargetTrait {}\npub fn new_api() { let x = 1; }\n",
        )
        .unwrap();

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
    fn grouped_use_member_addition_blocks_without_doc_update() {
        let tmp_dir = tempfile::tempdir().unwrap();
        let root = tmp_dir.path();
        setup_git_surface_repo(root, "pub use crate::plugins::{Alpha};\n");
        std::fs::write(
            root.join("src").join("server.rs"),
            "pub use crate::plugins::{Alpha, Beta};\n",
        )
        .unwrap();
        commit_all(root, "grouped use member addition");

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
    fn trait_method_signature_change_blocks_without_doc_update() {
        let tmp_dir = tempfile::tempdir().unwrap();
        let root = tmp_dir.path();
        setup_git_surface_repo(
            root,
            "pub trait TargetTrait {\n    fn fetch(&self, id: u64);\n}\n",
        );
        std::fs::write(
            root.join("src").join("server.rs"),
            "pub trait TargetTrait {\n    fn fetch(&self, id: String);\n}\n",
        )
        .unwrap();
        commit_all(root, "trait method signature change");

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
    fn struct_field_type_change_blocks_without_doc_update() {
        let tmp_dir = tempfile::tempdir().unwrap();
        let root = tmp_dir.path();
        setup_git_surface_repo(root, "pub struct Params {\n    pub id: u64,\n}\n");
        std::fs::write(
            root.join("src").join("server.rs"),
            "pub struct Params {\n    pub id: String,\n}\n",
        )
        .unwrap();
        commit_all(root, "struct field type change");

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
    fn enum_variant_change_blocks_without_doc_update() {
        let tmp_dir = tempfile::tempdir().unwrap();
        let root = tmp_dir.path();
        setup_git_surface_repo(root, "pub enum Thing {\n    A,\n    B,\n}\n");
        std::fs::write(
            root.join("src").join("server.rs"),
            "pub enum Thing {\n    A,\n    C,\n}\n",
        )
        .unwrap();
        commit_all(root, "enum variant change");

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
    fn struct_literal_reference_field_addition_passes_without_doc_update() {
        // Gate-level mirror of the live shape: adding a reference-value
        // field line to a struct literal inside a private function body
        // is not a public-surface change.
        let tmp_dir = tempfile::tempdir().unwrap();
        let root = tmp_dir.path();
        setup_git_surface_repo(
            root,
            "pub trait TargetTrait {}\nfn build() {\n    let params = Params {\n        principal: \"alice\",\n    };\n}\n",
        );
        std::fs::write(
            root.join("src").join("server.rs"),
            "pub trait TargetTrait {}\nfn build() {\n    let params = Params {\n        principal: \"alice\",\n        domains: &[],\n    };\n}\n",
        )
        .unwrap();
        commit_all(root, "struct literal field addition");

        let report = evaluate_doc_gate(root, None);
        assert!(
            report.passed,
            "struct-literal field addition produced violations: {:?}",
            report.violations
        );
        assert!(
            !report
                .violations
                .iter()
                .any(|v| v.kind == "surface_changed_without_doc_update"),
            "unexpected co-change violation: {:?}",
            report.violations
        );
    }

    #[test]
    fn bare_positional_argument_lines_pass_without_doc_update() {
        // Gate-level mirror of the full live shape, both stops in one
        // test: converting a multiline positional call to struct-literal
        // form and joining a multiline assert onto one line changes no
        // public surface — the removed side's bare `false,` / `statuses,`
        // lines and the added side's `domains: &[],` line extract nothing.
        let tmp_dir = tempfile::tempdir().unwrap();
        let root = tmp_dir.path();
        setup_git_surface_repo(
            root,
            "pub trait TargetTrait {}\nfn run() {\n    search_lexical(\n        \"ns\",\n        \"q\",\n        10,\n        false,\n        false,\n        &[],\n        \"alice\",\n        0,\n    );\n    assert_eq!(\n        statuses,\n        vec![1, 2]\n    );\n}\n",
        );
        std::fs::write(
            root.join("src").join("server.rs"),
            "pub trait TargetTrait {}\nfn run() {\n    search_lexical(crate::traits::SearchLexicalParams {\n        namespace: \"ns-batch-idx\",\n        query: \"zephyrquartz99\",\n        limit: 10,\n        include_tasks: false,\n        include_archived: false,\n        domains: &[],\n        principal: \"alice\",\n        fuzzy_max_distance: 0,\n    });\n    assert_eq!(statuses, vec![1, 2]);\n}\n",
        )
        .unwrap();
        commit_all(root, "positional call to struct literal");

        let report = evaluate_doc_gate(root, None);
        assert!(
            report.passed,
            "bare positional argument lines produced violations: {:?}",
            report.violations
        );
        assert!(
            !report
                .violations
                .iter()
                .any(|v| v.kind == "surface_changed_without_doc_update"),
            "unexpected co-change violation: {:?}",
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
    fn deleted_watched_file_blocks_without_doc_update() {
        let tmp_dir = tempfile::tempdir().unwrap();
        let root = tmp_dir.path();
        setup_git_surface_repo(root, "pub trait TargetTrait {}\n");
        run_git(root, &["rm", "-q", "src/server.rs"]);
        run_git(root, &["commit", "-q", "-m", "delete watched file"]);

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
    fn deleted_watched_file_with_doc_update_passes() {
        let tmp_dir = tempfile::tempdir().unwrap();
        let root = tmp_dir.path();
        setup_git_surface_repo(root, "pub trait TargetTrait {}\n");
        run_git(root, &["rm", "-q", "src/server.rs"]);
        std::fs::write(
            root.join("docs").join("vision").join("target_doc.md"),
            "## Introduction\nThe target_doc surface. TargetTrait removed.\n",
        )
        .unwrap();
        commit_all(root, "delete watched file with doc update");

        let report = evaluate_doc_gate(root, None);
        assert!(
            !report.violations.iter().any(|v| {
                v.kind == "surface_changed_without_doc_update" && v.rule_id == "test-rule-1"
            }),
            "deletion with doc update produced co-change violation: {:?}",
            report.violations
        );
    }

    #[test]
    fn deleted_internal_watched_file_passes_without_doc_update() {
        let tmp_dir = tempfile::tempdir().unwrap();
        let root = tmp_dir.path();
        setup_git_surface_repo(root, "fn helper() -> i32 { 1 }\n");
        run_git(root, &["rm", "-q", "src/server.rs"]);
        run_git(root, &["commit", "-q", "-m", "delete internal watched file"]);

        let report = evaluate_doc_gate(root, None);
        assert!(
            report.passed,
            "deleting internal-only file produced violations: {:?}",
            report.violations
        );
    }

    #[test]
    fn renamed_out_of_watch_with_surface_edit_blocks() {
        let tmp_dir = tempfile::tempdir().unwrap();
        let root = tmp_dir.path();
        setup_git_surface_repo(root, "pub trait TargetTrait {}\n");
        run_git(root, &["mv", "src/server.rs", "src/renamed.rs"]);
        std::fs::write(
            root.join("src").join("renamed.rs"),
            "pub trait RenamedTrait {}\n",
        )
        .unwrap();
        commit_all(root, "rename out of watch with surface edit");

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
    fn trait_method_change_within_existing_trait_differs() {
        let before = extract_public_surface_signatures("    fn fetch(&self, id: u64);");
        let after = extract_public_surface_signatures("    fn fetch(&self, id: String);");
        assert_ne!(before, after);
    }

    #[test]
    fn mcp_schema_property_and_required_changes_differ() {
        let before =
            extract_public_surface_signatures("                                \"query\": { \"type\": \"string\" },");
        let after =
            extract_public_surface_signatures("                                \"query\": { \"type\": \"integer\" },");
        assert_ne!(before, after, "before={before:?} after={after:?}");
        let before_req =
            extract_public_surface_signatures("                            \"required\": [\"content\"]");
        let after_req = extract_public_surface_signatures(
            "                            \"required\": [\"content\", \"namespace\"]",
        );
        assert_ne!(before_req, after_req);
    }

    #[test]
    fn serde_container_attribute_change_differs() {
        let before = extract_public_surface_signatures(
            "#[serde(tag = \"kind\")]\npub enum SubmissionCriterion {\n    Name(String),\n}",
        );
        let after = extract_public_surface_signatures(
            "#[serde(tag = \"type\")]\npub enum SubmissionCriterion {\n    Name(String),\n}",
        );
        assert_ne!(before, after, "before={before:?} after={after:?}");
        assert_ne!(
            before.get("SubmissionCriterion"),
            after.get("SubmissionCriterion")
        );
        let before = extract_public_surface_signatures(
            "#[serde(rename_all = \"camelCase\")]\npub struct Payload {\n    pub x: u32,\n}",
        );
        let after = extract_public_surface_signatures(
            "#[serde(rename_all = \"snake_case\")]\npub struct Payload {\n    pub x: u32,\n}",
        );
        assert_ne!(before, after, "before={before:?} after={after:?}");
    }

    #[test]
    fn serde_field_attribute_change_differs() {
        let before = extract_public_surface_signatures(
            "pub struct Payload {\n    #[serde(rename = \"a\")]\n    pub x: u32,\n}",
        );
        let after = extract_public_surface_signatures(
            "pub struct Payload {\n    #[serde(rename = \"b\")]\n    pub x: u32,\n}",
        );
        assert_ne!(before, after, "before={before:?} after={after:?}");
    }

    #[test]
    fn serde_variant_attribute_change_differs() {
        let before = extract_public_surface_signatures(
            "pub enum Kind {\n    #[serde(rename = \"a\")]\n    Alpha,\n}",
        );
        let after = extract_public_surface_signatures(
            "pub enum Kind {\n    #[serde(rename = \"b\")]\n    Alpha,\n}",
        );
        assert_ne!(before, after, "before={before:?} after={after:?}");
    }

    #[test]
    fn member_moved_between_structs_differs() {
        let before = extract_public_surface_signatures(
            "pub struct A {\n    pub x: u32,\n}\npub struct B {\n}",
        );
        let after = extract_public_surface_signatures(
            "pub struct A {\n}\npub struct B {\n    pub x: u32,\n}",
        );
        assert_ne!(before, after, "before={before:?} after={after:?}");
        assert!(before.contains_key("field:A.x"), "{before:?}");
        assert!(after.contains_key("field:B.x"), "{after:?}");
    }

    #[test]
    fn member_moved_between_traits_and_enums_differs() {
        let before = extract_public_surface_signatures(
            "pub trait A {\n    fn fetch(&self);\n}\npub trait B {\n}",
        );
        let after = extract_public_surface_signatures(
            "pub trait A {\n}\npub trait B {\n    fn fetch(&self);\n}",
        );
        assert_ne!(before, after, "before={before:?} after={after:?}");
        assert!(before.contains_key("fn:A.fetch"), "{before:?}");
        assert!(after.contains_key("fn:B.fetch"), "{after:?}");
        let before =
            extract_public_surface_signatures("pub enum A {\n    Alpha,\n}\npub enum B {\n}");
        let after =
            extract_public_surface_signatures("pub enum A {\n}\npub enum B {\n    Alpha,\n}");
        assert_ne!(before, after, "before={before:?} after={after:?}");
        assert!(before.contains_key("member:A.Alpha"), "{before:?}");
        assert!(after.contains_key("member:B.Alpha"), "{after:?}");
    }

    #[test]
    fn inherent_impl_method_moved_between_impls_differs() {
        // A `pub fn` in an inherent impl block is both a declaration
        // and a member: the declaration branch must not clear the
        // enclosing type before the member key is qualified with it.
        let before = extract_public_surface_signatures(
            "impl Foo {\n    pub fn bar(&self) {}\n}\nimpl Baz {\n}",
        );
        let after = extract_public_surface_signatures(
            "impl Foo {\n}\nimpl Baz {\n    pub fn bar(&self) {}\n}",
        );
        assert_ne!(before, after, "before={before:?} after={after:?}");
        assert!(before.contains_key("fn:Foo.bar"), "{before:?}");
        assert!(after.contains_key("fn:Baz.bar"), "{after:?}");
    }

    #[test]
    fn consecutive_impl_methods_all_qualify() {
        // A non-container declaration must not clear the enclosing
        // impl: every method in the block qualifies, not just the
        // first.
        let sigs = extract_public_surface_signatures(
            "impl Foo {\n    pub fn a(&self) {}\n    pub fn c(&self) {}\n}",
        );
        assert!(sigs.contains_key("fn:Foo.a"), "{sigs:?}");
        assert!(sigs.contains_key("fn:Foo.c"), "{sigs:?}");
    }

    #[test]
    fn second_method_moved_between_impls_differs() {
        // `bar` is the second method in both blocks: position must
        // not cancel the owner qualification.
        let before = extract_public_surface_signatures(
            "impl Foo {\n    pub fn a(&self) {}\n    pub fn bar(&self) {}\n}\nimpl Baz {\n    pub fn a(&self) {}\n}",
        );
        let after = extract_public_surface_signatures(
            "impl Foo {\n    pub fn a(&self) {}\n}\nimpl Baz {\n    pub fn a(&self) {}\n    pub fn bar(&self) {}\n}",
        );
        assert_ne!(before, after, "before={before:?} after={after:?}");
        assert!(before.contains_key("fn:Foo.bar"), "{before:?}");
        assert!(after.contains_key("fn:Baz.bar"), "{after:?}");
    }

    #[test]
    fn private_helper_after_pub_fn_in_impl_qualifies() {
        // A bare `fn` helper following a `pub fn` in one impl block
        // is still a member of that impl.
        let sigs = extract_public_surface_signatures(
            "impl Foo {\n    pub fn a(&self) {}\n    fn b(&self) {}\n}",
        );
        assert!(sigs.contains_key("fn:Foo.a"), "{sigs:?}");
        assert!(sigs.contains_key("fn:Foo.b"), "{sigs:?}");
    }

    #[test]
    fn top_level_fn_does_not_invent_owner_for_following_member() {
        // `enclosing` starts as None and a non-container declaration
        // must not invent an owner: a member line after a top-level
        // `pub fn` (no container in the text) keeps its bare key.
        let sigs = extract_public_surface_signatures("pub fn top() {}\n    pub fn bar(&self) {}");
        assert!(sigs.contains_key("fn:bar"), "{sigs:?}");
        assert!(!sigs.contains_key("fn:top.bar"), "{sigs:?}");
    }

    #[test]
    fn sequential_structs_still_scope_members_correctly() {
        // Keeping `enclosing` across non-container declarations must
        // not blur two sequential containers.
        let sigs = extract_public_surface_signatures(
            "pub struct A {\n    pub x: u32,\n    pub fn make() {}\n}\npub struct B {\n    pub x: String,\n}",
        );
        assert!(sigs.contains_key("field:A.x"), "{sigs:?}");
        assert!(sigs.contains_key("field:B.x"), "{sigs:?}");
    }

    #[test]
    fn inherent_impl_pub_fn_without_impl_keeps_bare_key() {
        // A `pub fn` member line with no preceding impl/container in
        // the text (the common -U0 diff shape) keeps its bare key.
        let sigs = extract_public_surface_signatures("    pub fn bar(&self) {}");
        assert!(sigs.contains_key("fn:bar"), "{sigs:?}");
    }

    #[test]
    fn trait_impl_methods_qualify_with_target_type() {
        // Trait-impl methods carry no `pub`: they already qualified,
        // and must keep doing so.
        let sigs =
            extract_public_surface_signatures("impl Greet for Foo {\n    fn bar(&self) {}\n}");
        assert!(sigs.contains_key("fn:Foo.bar"), "{sigs:?}");
        // A `pub fn` in an `impl Trait for Type` block qualifies with
        // the target type, not the trait.
        let sigs =
            extract_public_surface_signatures("impl Greet for Foo {\n    pub fn bar(&self) {}\n}");
        assert!(sigs.contains_key("fn:Foo.bar"), "{sigs:?}");
    }

    #[test]
    fn same_named_fields_in_two_structs_do_not_collide() {
        let sigs = extract_public_surface_signatures(
            "pub struct A {\n    pub x: u32,\n}\npub struct B {\n    pub x: String,\n}",
        );
        assert_eq!(sigs.get("field:A.x").map(|s| s.len()), Some(1), "{sigs:?}");
        assert_eq!(sigs.get("field:B.x").map(|s| s.len()), Some(1), "{sigs:?}");
    }

    #[test]
    fn member_without_enclosing_declaration_keeps_bare_key() {
        // A member line with no preceding container declaration in the
        // text (the common -U0 diff shape) keeps its bare key.
        let sigs = extract_public_surface_signatures("    fn fetch(&self, id: u64);");
        assert!(sigs.contains_key("fn:fetch"), "{sigs:?}");
        // The enclosing scope ends at the closing brace: a member
        // after it is not attributed to the closed container.
        let sigs = extract_public_surface_signatures(
            "pub struct A {\n    pub x: u32,\n}\n    pub y: u32,",
        );
        assert!(sigs.contains_key("field:A.x"), "{sigs:?}");
        assert!(sigs.contains_key("field:y"), "{sigs:?}");
    }

    #[test]
    fn serde_attribute_not_preceding_a_declaration_is_ignored() {
        // A serde-shaped attribute that does not directly precede a
        // declaration or member (here it sits over a `let` inside a fn
        // body) is not item surface and must not register.
        let before = extract_public_surface_signatures(
            "pub fn helper() {\n    #[serde(tag = \"a\")]\n    let x = 1;\n}",
        );
        let after = extract_public_surface_signatures(
            "pub fn helper() {\n    #[serde(tag = \"b\")]\n    let x = 1;\n}",
        );
        assert_eq!(before, after, "before={before:?} after={after:?}");
    }

    #[test]
    fn command_alias_change_differs() {
        // W3 probe (Codex PR #162): the alias on the synaptic-mesh
        // command declaration is CLI surface, but only the command
        // name entered the map (via SS7), so an alias-only change
        // produced identical maps.
        let before = extract_public_surface_signatures(
            "#[command(name = \"synaptic-mesh\", alias = \"mesh\")]\npub struct SynapticMesh;",
        );
        let after = extract_public_surface_signatures(
            "#[command(name = \"synaptic-mesh\", alias = \"connector\")]\npub struct SynapticMesh;",
        );
        assert_ne!(before, after, "before={before:?} after={after:?}");
        assert_ne!(
            before.get("SynapticMesh"),
            after.get("SynapticMesh"),
            "attribute must enter the enclosing declaration's signature: {before:?} vs {after:?}"
        );
    }

    #[test]
    fn command_alias_only_attribute_change_differs() {
        let before =
            extract_public_surface_signatures("#[command(alias = \"mesh\")]\npub struct Thing;");
        let after = extract_public_surface_signatures(
            "#[command(alias = \"connector\")]\npub struct Thing;",
        );
        assert_ne!(before, after, "before={before:?} after={after:?}");
        // The attribute line alone (the -U0 diff shape for an
        // alias-only edit, where the annotated variant line itself
        // is unchanged context) must already register.
        let before_line =
            extract_public_surface_signatures("#[command(alias = \"mesh\")]");
        let after_line =
            extract_public_surface_signatures("#[command(alias = \"connector\")]");
        assert_ne!(
            before_line, after_line,
            "before={before_line:?} after={after_line:?}"
        );
    }

    #[test]
    fn command_visible_alias_change_differs() {
        let before = extract_public_surface_signatures(
            "#[command(name = \"x\", visible_alias = \"a\")]\npub struct Thing;",
        );
        let after = extract_public_surface_signatures(
            "#[command(name = \"x\", visible_alias = \"b\")]\npub struct Thing;",
        );
        assert_ne!(before, after, "before={before:?} after={after:?}");
    }

    #[test]
    fn command_attribute_not_preceding_a_declaration_is_ignored_for_attach() {
        // Placement parity with serde: an attribute over a `let`
        // inside a fn body does not attach to a declaration. (The
        // shared `command` key still records the attribute itself,
        // following the clap pattern, so both sides below carry it
        // identically only when the attribute is unchanged.)
        let before = extract_public_surface_signatures(
            "pub fn helper() {\n    #[command(alias = \"a\")]\n    let x = 1;\n}",
        );
        assert!(
            !before.contains_key("helper")
                || !before["helper"].iter().any(|s| s.starts_with("command(")),
            "attribute attached to a non-declaration: {before:?}"
        );
    }

    #[test]
    fn test_fn_rename_in_cfg_test_mod_is_identical() {
        // X1 probe (Codex PR #162): renaming ONLY a test function in
        // src/traits.rs changed the signature map, so a test-only
        // change demanded a documentation update. Test functions are
        // not public surface.
        let before = extract_public_surface_signatures(
            "#[cfg(test)]\nmod tests {\n    #[test]\n    fn memory_vocabulary_declares_task() {\n    }\n}",
        );
        let after = extract_public_surface_signatures(
            "#[cfg(test)]\nmod tests {\n    #[test]\n    fn memory_vocabulary_declares_renamed() {\n    }\n}",
        );
        assert_eq!(before, after, "before={before:?} after={after:?}");
        let decls = extract_public_surface_declarations(
            "#[cfg(test)]\nmod tests {\n    #[test]\n    fn memory_vocabulary_declares_task() {\n    }\n}",
        );
        assert!(
            !decls.contains("memory_vocabulary_declares_task"),
            "{decls:?}"
        );
    }

    #[test]
    fn test_helper_in_cfg_test_mod_signature_change_is_identical() {
        // A helper fn inside a #[cfg(test)] module carries no
        // #[test] attribute of its own; the enclosing test scope
        // still makes it test-only.
        let before = extract_public_surface_signatures(
            "#[cfg(test)]\nmod tests {\n    fn helper(value: u32) -> u32 {\n        value\n    }\n}",
        );
        let after = extract_public_surface_signatures(
            "#[cfg(test)]\nmod tests {\n    fn helper(value: String) -> u32 {\n        0\n    }\n}",
        );
        assert_eq!(before, after, "before={before:?} after={after:?}");
    }

    #[test]
    fn test_attribute_fn_outside_a_module_is_identical() {
        // A #[test] fn at file scope (no enclosing test module) is
        // test-only too, as is a #[cfg(test)] helper item.
        let before =
            extract_public_surface_signatures("#[test]\nfn it_works() {\n}");
        let after =
            extract_public_surface_signatures("#[test]\nfn it_passes() {\n}");
        assert_eq!(before, after, "before={before:?} after={after:?}");
        let before = extract_public_surface_signatures(
            "#[cfg(test)]\npub fn test_helper(x: u32) {}",
        );
        let after = extract_public_surface_signatures(
            "#[cfg(test)]\npub fn test_helper(x: String) {}",
        );
        assert_eq!(before, after, "before={before:?} after={after:?}");
    }

    #[test]
    fn mcp_schema_required_change_in_one_tool_differs() {
        // X2 probe (Codex PR #162): dropping "namespace" from
        // neurostrata_get_memory's required array in src/server.rs
        // produced identical maps, because the same array occurs in
        // other tools and every entry aggregated under the shared
        // `mcp-schema:required` key.
        let tool = |name: &str, required: &str| {
            format!(
                "{{\n    \"name\": \"{name}\",\n    \"inputSchema\": {{\n        \"type\": \"object\",\n        \"properties\": {{\n            \"id\": {{ \"type\": \"string\" }},\n            \"namespace\": {{ \"type\": \"string\" }}\n        }},\n        \"required\": {required}\n    }}\n}}"
            )
        };
        let before = extract_public_surface_signatures(&format!(
            "{},\n{}",
            tool("neurostrata_get_memory", "[\"id\", \"namespace\"]"),
            tool("neurostrata_other_tool", "[\"id\", \"namespace\"]")
        ));
        let after = extract_public_surface_signatures(&format!(
            "{},\n{}",
            tool("neurostrata_get_memory", "[\"id\"]"),
            tool("neurostrata_other_tool", "[\"id\", \"namespace\"]")
        ));
        assert_ne!(before, after, "before={before:?} after={after:?}");
        assert!(
            before.contains_key("mcp-schema:neurostrata_get_memory:required"),
            "{before:?}"
        );
    }

    #[test]
    fn mcp_schema_type_change_in_one_tool_differs() {
        // The standalone `"type":` lines of two tools likewise
        // aggregated under the shared `mcp-schema:type` key: changing
        // one property's type to a value another property already
        // used cancelled out.
        let tool = |name: &str, id_type: &str| {
            format!(
                "{{\n    \"name\": \"{name}\",\n    \"inputSchema\": {{\n        \"properties\": {{\n            \"id\": {{\n                \"type\": \"{id_type}\"\n            }},\n            \"count\": {{\n                \"type\": \"integer\"\n            }}\n        }}\n    }}\n}}"
            )
        };
        let before = extract_public_surface_signatures(&format!(
            "{},\n{}",
            tool("neurostrata_get_memory", "string"),
            tool("neurostrata_other_tool", "string")
        ));
        let after = extract_public_surface_signatures(&format!(
            "{},\n{}",
            tool("neurostrata_get_memory", "integer"),
            tool("neurostrata_other_tool", "string")
        ));
        assert_ne!(before, after, "before={before:?} after={after:?}");
        assert!(
            before.contains_key("mcp-schema:neurostrata_get_memory:id.type"),
            "{before:?}"
        );
    }

    #[test]
    fn mcp_schema_nested_required_is_qualified_by_property_path() {
        // A `required` array nested inside a property's items must
        // be qualified by the property path as well as the tool, or
        // it cancels against the same nested shape in another tool.
        let tool = |name: &str, required: &str| {
            format!(
                "{{\n    \"name\": \"{name}\",\n    \"inputSchema\": {{\n        \"properties\": {{\n            \"candidates\": {{\n                \"type\": \"array\",\n                \"items\": {{\n                    \"properties\": {{\n                        \"title\": {{ \"type\": \"string\" }}\n                    }},\n                    \"required\": {required}\n                }}\n            }}\n        }}\n    }}\n}}"
            )
        };
        let before = extract_public_surface_signatures(&format!(
            "{},\n{}",
            tool("neurostrata_task_create", "[\"title\"]"),
            tool("neurostrata_other_tool", "[\"title\"]")
        ));
        let after = extract_public_surface_signatures(&format!(
            "{},\n{}",
            tool("neurostrata_task_create", "[]"),
            tool("neurostrata_other_tool", "[\"title\"]")
        ));
        assert_ne!(before, after, "before={before:?} after={after:?}");
        assert!(
            before.contains_key(
                "mcp-schema:neurostrata_task_create:candidates.items.required"
            ),
            "{before:?}"
        );
    }

    #[test]
    fn clap_arg_attribute_change_differs() {
        let before = extract_public_surface_signatures("        #[arg(long)]");
        let after = extract_public_surface_signatures("        #[arg(short)]");
        assert_ne!(before, after, "before={before:?} after={after:?}");
    }

    #[test]
    fn clap_arg_change_on_one_variant_field_differs() {
        // X3 probe (Codex PR #162): changing the daemon `profile`
        // option in src/main.rs from #[arg(long)] to
        // #[arg(short, long)] produced identical maps, because every
        // #[arg(...)] aggregated under the shared `clap-arg` key and
        // both spellings occur on other options.
        let before = extract_public_surface_signatures(
            "enum Commands {\n    Daemon {\n        #[arg(long)]\n        profile: bool,\n    },\n    Other {\n        #[arg(short, long)]\n        verbose: bool,\n        #[arg(long)]\n        force: bool,\n    },\n}",
        );
        let after = extract_public_surface_signatures(
            "enum Commands {\n    Daemon {\n        #[arg(short, long)]\n        profile: bool,\n    },\n    Other {\n        #[arg(short, long)]\n        verbose: bool,\n        #[arg(long)]\n        force: bool,\n    },\n}",
        );
        assert_ne!(before, after, "before={before:?} after={after:?}");
        assert!(
            before.contains_key("clap-arg:Commands.Daemon.profile"),
            "{before:?}"
        );
    }

    #[test]
    fn clap_arg_change_on_one_struct_field_differs() {
        // Same shared-key cancellation between two Args structs:
        // the attribute must be qualified by its owning struct and
        // field.
        let before = extract_public_surface_signatures(
            "pub struct Args {\n    #[arg(long)]\n    pub profile: bool,\n}\npub struct Other {\n    #[arg(short, long)]\n    pub profile: bool,\n    #[arg(long)]\n    pub force: bool,\n}",
        );
        let after = extract_public_surface_signatures(
            "pub struct Args {\n    #[arg(short, long)]\n    pub profile: bool,\n}\npub struct Other {\n    #[arg(short, long)]\n    pub profile: bool,\n    #[arg(long)]\n    pub force: bool,\n}",
        );
        assert_ne!(before, after, "before={before:?} after={after:?}");
        assert!(before.contains_key("clap-arg:Args.profile"), "{before:?}");
    }

    #[test]
    fn multiline_route_method_continuation_matches_documented_limit() {
        let before = extract_public_surface_signatures(
            "            get(handle_http_storage_status).post(handle_http_storage_status),",
        );
        let after =
            extract_public_surface_signatures("            get(handle_http_storage_status),");
        assert_eq!(before, after);
    }

    #[test]
    fn typescript_interface_member_change_differs() {
        let before = extract_public_surface_signatures("  strictMode: boolean");
        let after = extract_public_surface_signatures("  strictMode: string");
        assert_ne!(before, after, "before={before:?} after={after:?}");
    }

    #[test]
    fn legacy_manifest_field_alias_still_enables_co_change_rule() {
        let json = r#"{
            "schema_version": 1,
            "tiers": {
                "vision": {"dir": "vision", "description": "v"},
                "features_api": {"dir": "features", "description": "f"},
                "extend": {"dir": "extend", "description": "e"}
            },
            "surface_rules": [{
                "id": "r1", "tier": "features_api", "target_doc": "features/x.md",
                "require_co_change_on_touch": true
            }]
        }"#;
        let manifest: DocManifest = serde_json::from_str(json).unwrap();
        assert!(manifest.surface_rules[0].require_co_change_on_surface_change);
    }

    #[test]
    fn restricted_visibility_items_are_not_public_surface() {
        let decls = extract_public_surface_declarations("pub(crate) enum ScopedEnum { A }");
        assert!(!decls.contains("ScopedEnum"), "{decls:?}");
        let sigs = extract_public_surface_signatures("pub(crate) fn internal_op() {}");
        assert!(sigs.is_empty(), "{sigs:?}");
        let sigs_field =
            extract_public_surface_signatures("    pub(crate) count: u64,");
        assert!(sigs_field.is_empty(), "{sigs_field:?}");
    }

    #[test]
    fn reexported_restricted_fn_signature_change_differs() {
        let before = extract_public_surface_signatures(
            "pub(crate) fn internal_op(x: u32) {}\npub use crate::m::internal_op;",
        );
        let after = extract_public_surface_signatures(
            "pub(crate) fn internal_op(x: String) {}\npub use crate::m::internal_op;",
        );
        assert_ne!(before, after, "before={before:?} after={after:?}");
    }

    #[test]
    fn reexported_restricted_fn_declaration_is_surface_under_rename() {
        // The use line surfaces only the alias; the definition itself is
        // surface too (compare a bare-`pub` fn re-exported under a rename,
        // whose own name enters the set from its declaration line).
        let decls = extract_public_surface_declarations(
            "pub(crate) fn internal_op(x: u32) {}\npub use crate::m::internal_op as external_op;",
        );
        assert!(decls.contains("internal_op"), "{decls:?}");
        assert!(decls.contains("external_op"), "{decls:?}");
        let before = extract_public_surface_signatures(
            "pub(crate) fn internal_op(x: u32) {}\npub use crate::m::internal_op as external_op;",
        );
        let after = extract_public_surface_signatures(
            "pub(crate) fn internal_op(x: String) {}\npub use crate::m::internal_op as external_op;",
        );
        assert_ne!(before, after, "before={before:?} after={after:?}");
    }

    #[test]
    fn grouped_reexport_restricted_fn_signature_change_differs() {
        let before = extract_public_surface_signatures(
            "pub(crate) fn internal_op(x: u32) {}\npub use crate::m::{internal_op, other};",
        );
        let after = extract_public_surface_signatures(
            "pub(crate) fn internal_op(x: String) {}\npub use crate::m::{internal_op, other};",
        );
        assert_ne!(before, after, "before={before:?} after={after:?}");
    }

    #[test]
    fn unreexported_restricted_fn_stays_excluded() {
        let decls = extract_public_surface_declarations("pub(crate) fn internal_op(x: u32) {}");
        assert!(!decls.contains("internal_op"), "{decls:?}");
        let sigs = extract_public_surface_signatures("pub(crate) fn internal_op(x: u32) {}");
        assert!(!sigs.contains_key("internal_op"), "{sigs:?}");
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
