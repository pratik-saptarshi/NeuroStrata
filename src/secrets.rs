use std::sync::OnceLock;

use regex::Regex;
use serde_json::Value;

/// Rejection reason produced by the scanner.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rejection {
    /// Which field was rejected, e.g. "content", "metadata.governs".
    pub location: String,
    /// Category of the detected secret, e.g. "provider_token", "jwt", "password".
    pub category: &'static str,
}

impl std::fmt::Display for Rejection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "ERROR [SECURITY]: Rejected {} ({}). Do not store secrets.",
            self.location, self.category
        )
    }
}

/// The compiled scanner: a set of pre-compiled regexes and a set of provider
/// form recognisers. Built once and reused across every request.
pub(crate) struct SecretScanner {
    provider_re: Regex,
    kv_re: Regex,
    jwt_re: Regex,
    he_re: Regex,
}

/// Provider token prefixes and their expected shape (alphabet + min length
/// after the prefix). The scanner checks the value side of key=value pairs,
/// but also standalone tokens that match a known prefix.
struct ProviderForm {
    prefix: &'static str,
    /// Minimum length of the whole token (prefix + suffix).
    min_len: usize,
    /// If set, the suffix must match this regex (which anchors at ^).
    suffix_re: Option<&'static str>,
}

const PROVIDER_FORMS: &[ProviderForm] = &[
    ProviderForm {
        // GitHub PAT: ghp_, gho_, ghs_, ghr_, github_pat_
        // Real tokens are 40+ chars (prefix + 36-38 base62). 16 catches
        // partial leaks (e.g. test fixtures) while excluding trivial matches.
        prefix: "ghp_",
        min_len: 16,
        suffix_re: None,
    },
    ProviderForm {
        prefix: "gho_",
        min_len: 16,
        suffix_re: None,
    },
    ProviderForm {
        prefix: "ghs_",
        min_len: 16,
        suffix_re: None,
    },
    ProviderForm {
        prefix: "ghr_",
        min_len: 16,
        suffix_re: None,
    },
    ProviderForm {
        prefix: "github_pat_",
        min_len: 40,
        suffix_re: None,
    },
    ProviderForm {
        // Anthropic: sk-ant-api03-... (base62, long)
        prefix: "sk-ant-",
        min_len: 16,
        suffix_re: None,
    },
    ProviderForm {
        // OpenAI sk- (not sk-proj-, handled separately): base62, ~51 chars total
        prefix: "sk-proj-",
        min_len: 20,
        suffix_re: None,
    },
    ProviderForm {
        // OpenAI sk- must come AFTER sk-proj- and sk-ant- to avoid
        // false-positive prefix overlap. Higher min_len because sk- is
        // a common prefix; real keys are ~51 chars.
        prefix: "sk-",
        min_len: 40,
        suffix_re: None,
    },
    ProviderForm {
        // Slack xox[baprs]-
        prefix: "xoxb-",
        min_len: 16,
        suffix_re: None,
    },
    ProviderForm {
        prefix: "xoxp-",
        min_len: 16,
        suffix_re: None,
    },
    ProviderForm {
        prefix: "xoxa-",
        min_len: 16,
        suffix_re: None,
    },
    ProviderForm {
        prefix: "xoxr-",
        min_len: 16,
        suffix_re: None,
    },
    ProviderForm {
        prefix: "xoxs-",
        min_len: 16,
        suffix_re: None,
    },
    ProviderForm {
        // AWS access key: AKIA followed by exactly 16 uppercase alphanumeric
        prefix: "AKIA",
        min_len: 20,
        suffix_re: Some("^[0-9A-Z]{16}$"),
    },
];

/// Patterns that look like placeholders or documentation fixtures and must
/// NOT be treated as secrets, even if they match a provider prefix or key.
/// Checked as whole-value or prefix matches, not loose substrings, to avoid
/// false positives on tokens that happen to contain common words.
const PLACEHOLDER_PATTERNS: &[&str] = &[
    "changeme",
    "your-api-key-here",
    "your_key_here",
    "your-secret-here",
    "replace-me",
    "replace_me",
    "insert-key",
    "insert_key",
    "placeholder",
];

/// Maximum recursion depth for metadata scanning.
const MAX_DEPTH: usize = 8;
/// Maximum total characters scanned per top-level call.
const MAX_SCAN_BYTES: usize = 256_000;
/// Overlap between scan chunks so a token that straddles a chunk boundary is
/// still seen whole. Provider tokens are far shorter than this.
const SCAN_CHUNK_OVERLAP: usize = 64 * 1024;
/// Carry a little context across a line boundary so a key on one line and its
/// value on the next are still scanned together. Tokens themselves never span
/// a newline, so this is small; the 64 KB overlap is only for long single
/// lines with no boundary to use.
const SCAN_CONTEXT_CARRY: usize = 4 * 1024;

/// A value captured next to a secret-like key is a source expression, not a
/// credential literal, when it is unquoted and shaped like code. Skipping
/// these avoids treating `let token = read_token();` as a password while
/// quoted literals and plain opaque values still go through the password
/// check.
fn looks_like_code_expression(value: &str) -> bool {
    // Strong source markers: path/call syntax and code-only roots. A config
    // value such as `password=correct.horse` does not qualify; member chains
    // and index expressions need a source-statement prefix, below.
    value.contains("::")
        || value.contains("->")
        || value.contains("=>")
        || (value.contains('(') && value.contains(')'))
        || value.starts_with("self.")
        || value.starts_with("&self.")
        || value.starts_with("process.env.")
}

/// Member chains and index expressions are exempt only inside a source
/// statement: `let token = response.token` yes, `password=correct.horse` no.
fn looks_like_source_reference(value: &str) -> bool {
    (!value.starts_with("eyJ") && looks_like_member_chain(value))
        || (value.contains('[') && !value.starts_with('[') && !value.contains(']'))
}

/// True when the key begins a source statement/comment, so code-shaped values
/// can be exempted without exempting the same shapes in a `.env` file.
fn source_statement_prefix(text: &str, key_start: usize) -> bool {
    let line_start = text[..key_start].rfind('\n').map(|i| i + 1).unwrap_or(0);
    let prefix = text[line_start..key_start].trim_start();
    [
        "let ", "const ", "var ", "return ", "if ", "match ", "for ", "while ", "pub ", "fn ",
        "struct ", "enum ", "impl ", "type ",
    ]
    .iter()
    .any(|p| prefix.starts_with(p))
}

/// `self.token`, `response.token`, `process.env.SECRET` are member chains; a
/// password such as `p@ss.w0rd` is not because a segment contains `@`.
fn looks_like_member_chain(value: &str) -> bool {
    if !value.contains('.') {
        return false;
    }
    let mut saw_alpha = false;
    for segment in value.split('.') {
        if segment.is_empty()
            || !segment
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_')
        {
            return false;
        }
        if let Some(first) = segment.chars().next() {
            saw_alpha |= first.is_ascii_alphabetic();
        }
    }
    saw_alpha
}

/// A value captured next to a secret-like key may be a Rust type annotation
/// (`api_key: String`, `token: Option<PathBuf>`) rather than a credential.
fn is_known_type_name(value: &str) -> bool {
    const TYPES: &[&str] = &[
        "String",
        "str",
        "bool",
        "usize",
        "u64",
        "u32",
        "u16",
        "u8",
        "i64",
        "i32",
        "i16",
        "i8",
        "f64",
        "f32",
        "PathBuf",
        "Path",
        "Option",
        "Result",
        "Vec",
        "Arc",
        "Rc",
        "Box",
        "Mutex",
        "RwLock",
        "SecretString",
    ];
    TYPES.iter().any(|ty| {
        value == *ty
            || value
                .strip_prefix(ty)
                .map(|rest| rest.starts_with('<') || rest.starts_with("::"))
                .unwrap_or(false)
    })
}

fn looks_like_type_annotation(value: &str) -> bool {
    let v = value.trim();
    if v.is_empty() {
        return false;
    }
    if v.contains('<') && v.contains('>') {
        return true;
    }
    if let Some(rest) = v.strip_prefix('&') {
        let mut rest = rest.trim_start();
        if let Some(after_quote) = rest.strip_prefix('\'') {
            rest = after_quote
                .split_once(' ')
                .map(|(_, r)| r.trim_start())
                .unwrap_or(after_quote);
        }
        rest = rest.strip_prefix("mut ").unwrap_or(rest);
        if let Some(inner) = rest.strip_prefix('[') {
            return inner
                .split([';', ']'])
                .next()
                .map(|t| is_known_type_name(t.trim()))
                .unwrap_or(false);
        }
        if let Some(dynamic) = rest.strip_prefix("dyn ") {
            return dynamic
                .chars()
                .next()
                .map(|c| c.is_ascii_uppercase())
                .unwrap_or(false);
        }
        return is_known_type_name(rest);
    }
    if let Some(inner) = v.strip_prefix('[') {
        return inner
            .split([';', ']'])
            .next()
            .map(|t| is_known_type_name(t.trim()))
            .unwrap_or(false);
    }
    if let Some(dynamic) = v.strip_prefix("dyn ") {
        return dynamic
            .chars()
            .next()
            .map(|c| c.is_ascii_uppercase())
            .unwrap_or(false);
    }
    is_known_type_name(v)
}

impl SecretScanner {
    /// Build the scanner. Returns `Err` if a regex fails to compile.
    fn build() -> Result<Self, regex::Error> {
        // Provider token form: word-boundary anchored, full-match check is done
        // separately after the regex finds a candidate.
        let provider_re = Regex::new(
            r"(?i)\b(?:sk-ant-|ghp_|gho_|ghs_|ghr_|github_pat_|sk-proj-|sk-|xox[baprs]-|AKIA)[A-Za-z0-9_\-]{4,}\b",
        )?;

        // Key=value candidate extraction. Matches:
        //   key = "value"      key : value
        //   "key": "value"     key=value
        // Case-insensitive keys. Allows optional quotes around the key name
        // (for JSON-style) and around the value. No backreferences.
        let kv_re = Regex::new(
            r#"(?i)["']?\s*(api[_-]?key|apikey|secret|token|password|passwd|private[_-]?key|client[_-]?secret|access[_-]?token|refresh[_-]?token|bearer)\s*["']?\s*[=:]\s*["']?([^"'\s,;}\]]{4,})"#,
        )?;

        // JWT: three base64url segments separated by dots.
        let jwt_re = Regex::new(r"eyJ[A-Za-z0-9_\-]+\.eyJ[A-Za-z0-9_\-]+\.[A-Za-z0-9_\-]+")?;

        // High-entropy assignment check: secret-like keys (secret, password,
        // passwd, api_key, apikey, access_key, client_secret, private_key)
        // followed by '=' or ':' and a long opaque value (no whitespace, no
        // path separators). Entropy/class-length validation happens in Rust.
        let he_re = Regex::new(
            r#"(?i)(?:secret|password|passwd|api[_-]?key|access[_-]?key|client[_-]?secret|private[_-]?key)["']?\s*[=:]\s*["']?([^"'\s\\/]{24,})"#,
        )?;

        Ok(Self {
            provider_re,
            kv_re,
            jwt_re,
            he_re,
        })
    }

    fn instance() -> &'static SecretScanner {
        static SCANNER: OnceLock<SecretScanner> = OnceLock::new();
        // Fail closed: if a built-in regex fails to compile this must panic at
        // first use, never silently disable the scanner.
        SCANNER.get_or_init(|| {
            SecretScanner::build().expect("built-in secret scanner regexes must compile")
        })
    }

    /// Scan a string for secrets. Returns `None` if clean or the scanner is
    /// disabled, `Some(Rejection)` with category + location on detection.
    pub fn scan_str(text: &str, location: &str) -> Option<Rejection> {
        let scanner = Self::instance();
        scanner.scan_chunked(text, location)
    }

    /// Scan arbitrarily large text in overlapping chunks. A whole-file
    /// rejection for size would silently discard large clean source files
    /// (this repository's `src/server.rs` is ~684 KB); secrets are found
    /// where they are, and a clean large file is admitted.
    fn scan_chunked(&self, text: &str, location: &str) -> Option<Rejection> {
        if text.len() <= MAX_SCAN_BYTES {
            return self.scan_inner(text, location);
        }
        let mut start = 0usize;
        while start < text.len() {
            let mut end = (start + MAX_SCAN_BYTES).min(text.len());
            while end > start && !text.is_char_boundary(end) {
                end -= 1;
            }
            if end <= start {
                break;
            }
            // Prefer a line boundary: a JWT or provider token never spans a
            // newline, so ending the chunk at one keeps the whole candidate in
            // the next chunk regardless of its length.
            let mut used_newline = false;
            if end < text.len() {
                if let Some(nl) = text[start..end].rfind('\n') {
                    let candidate = start + nl + 1;
                    if candidate - start > MAX_SCAN_BYTES / 2 {
                        end = candidate;
                        used_newline = true;
                    }
                }
            }
            if let Some(r) = self.scan_inner(&text[start..end], location) {
                return Some(r);
            }
            if end == text.len() {
                break;
            }
            if used_newline {
                // Keep a little context so `key =` at the end of one chunk and
                // its value on the next line still form a kv_re match.
                let next = end.saturating_sub(SCAN_CONTEXT_CARRY);
                start = if next > start { next } else { end };
            } else {
                let next = end.saturating_sub(SCAN_CHUNK_OVERLAP);
                start = if next > start { next } else { end };
            }
            while start < text.len() && !text.is_char_boundary(start) {
                start += 1;
            }
        }
        None
    }

    /// Scan a JSON value recursively (metadata leaves), returning the first
    /// rejection found.
    pub fn scan_value(value: &Value, location: &str) -> Option<Rejection> {
        let scanner = Self::instance();
        scanner.scan_value_inner(value, location, 0)
    }

    fn scan_inner(&self, text: &str, location: &str) -> Option<Rejection> {
        if text.len() > MAX_SCAN_BYTES {
            return Some(Rejection {
                location: location.to_string(),
                category: "oversized_input",
            });
        }

        // --- JWT detection ---
        if let Some(cat) = self.detect_jwt(text) {
            return Some(Rejection {
                location: location.to_string(),
                category: cat,
            });
        }

        // --- Provider token form detection ---
        // Try the provider regex on the whole text and on each extracted
        // value-side from key=value pairs.
        for mat in self.provider_re.find_iter(text) {
            let candidate = mat.as_str();
            if let Some(cat) = self.validate_provider_form(candidate) {
                if !is_placeholder(&candidate.to_ascii_lowercase()) {
                    return Some(Rejection {
                        location: location.to_string(),
                        category: cat,
                    });
                }
            }
        }

        // --- High-entropy assignment-style secrets ---
        // Runs BEFORE the key=value phase so genuinely high-entropy opaque
        // values next to secret-like keys get the precise
        // `high_entropy_secret` detector; shorter/low-entropy values still
        // fall through to the weak-password check below.
        for cap in self.he_re.captures_iter(text) {
            let token = cap.get(1).map(|m| m.as_str()).unwrap_or("");
            if is_placeholder(&token.to_ascii_lowercase()) {
                continue;
            }
            if meets_high_entropy(token) {
                return Some(Rejection {
                    location: location.to_string(),
                    category: "high_entropy_secret",
                });
            }
        }

        // --- Key=value extraction ---
        for cap in self.kv_re.captures_iter(text) {
            let value = cap.get(2).map(|m| m.as_str()).unwrap_or("");
            let value_lower = value.to_ascii_lowercase();

            if is_placeholder(&value_lower) {
                continue;
            }

            // The capture stops at `;`, so a quoted value containing `;` has no
            // closing quote at `m.end()`; the opening quote is the reliable
            // marker that this is a literal.
            let quoted = cap
                .get(2)
                .map(|m| matches!(text[..m.start()].chars().last(), Some('"') | Some('\'')))
                .unwrap_or(false);
            // Type annotations use `:`. A config value after `=` (for example
            // `password=&hunter2!`) is never a type annotation, even if it
            // starts with a reference sigil.
            let type_context = cap
                .get(1)
                .zip(cap.get(2))
                .map(|(key, val)| {
                    let sep = text[key.end()..val.start()].trim();
                    sep.starts_with(':') && !sep.starts_with("::")
                })
                .unwrap_or(false);
            let source_context = cap
                .get(1)
                .map(|key| source_statement_prefix(text, key.start()))
                .unwrap_or(false);
            let literal_keyword =
                matches!(value, "None" | "true" | "false") || value.starts_with("&self.");
            if !quoted
                && (literal_keyword
                    || looks_like_code_expression(value)
                    || (source_context && looks_like_source_reference(value))
                    || (type_context && looks_like_type_annotation(value)))
            {
                continue;
            }

            // Weak password detection: next to password=, even low-entropy
            // values like "hunter2" must be caught. Check if this value looks
            // like a real password (>=4 chars, not all same char).
            if value.len() >= 4
                && !value
                    .chars()
                    .all(|c| c == value.chars().next().unwrap_or('x'))
            {
                return Some(Rejection {
                    location: location.to_string(),
                    category: "password",
                });
            }
        }

        // --- Generic high-entropy check on remaining opaque values ---
        // Only for the direct text scan (not metadata leaves — those go
        // through scan_value which handles strings individually).

        None
    }

    fn scan_value_inner(&self, value: &Value, location: &str, depth: usize) -> Option<Rejection> {
        scan_value_with(
            &|text, loc| self.scan_inner(text, loc),
            value,
            location,
            depth,
        )
    }

    /// Detect a JWT-shaped token: three base64url segments where segment 1
    /// decodes to a JSON object containing `alg` and `typ` (case-sensitive
    /// bytes).
    fn detect_jwt(&self, text: &str) -> Option<&'static str> {
        // Find all dot-separated triplets that look base64url.
        // We look for the pattern: non-dot+ . non-dot+ . non-dot+
        for mat in self.jwt_re.find_iter(text) {
            let jwt_str = mat.as_str();
            // Validate it's actually a JWT by decoding segment 1.
            if let Some(dots) = find_jwt_segments(jwt_str) {
                if is_valid_jwt_header(dots.0) {
                    return Some("jwt");
                }
            }
        }
        None
    }

    /// Validate a candidate against known provider form rules.
    /// Returns the category if it matches, None if it's a false positive.
    fn validate_provider_form(&self, candidate: &str) -> Option<&'static str> {
        let lower = candidate.to_ascii_lowercase();
        for form in PROVIDER_FORMS {
            if lower.starts_with(&form.prefix.to_ascii_lowercase()) {
                if candidate.len() < form.min_len {
                    // Truncated prefix — not a real token.
                    continue;
                }
                if let Some(suffix_pattern) = form.suffix_re {
                    // Extract the part after the prefix.
                    let suffix = &candidate[form.prefix.len()..];
                    if let Ok(re) = Regex::new(suffix_pattern) {
                        if !re.is_match(suffix) {
                            continue;
                        }
                    }
                }
                // Determine category from prefix.
                return Some(
                    if lower.starts_with("ghp_")
                        || lower.starts_with("gho_")
                        || lower.starts_with("ghs_")
                        || lower.starts_with("ghr_")
                        || lower.starts_with("github_pat_")
                    {
                        "github_pat"
                    } else if lower.starts_with("sk-ant-") {
                        "anthropic_token"
                    } else if lower.starts_with("sk-proj-") {
                        "openai_project_key"
                    } else if lower.starts_with("sk-") {
                        "openai_key"
                    } else if lower.starts_with("xox") {
                        "slack_token"
                    } else if lower.starts_with("akia") {
                        "aws_access_key"
                    } else {
                        "provider_token"
                    },
                );
            }
        }
        None
    }
}

/// Traverse a JSON value, using `scan_text` for every string leaf and object
/// key. `SecretScanner::scan_value_inner` delegates here with its real
/// `scan_inner`; the Kani Tier 2 harnesses pass a registered oracle instead,
/// so the traversal can be proved without fabricating a `SecretScanner`.
fn scan_value_with<F>(
    scan_text: &F,
    value: &Value,
    location: &str,
    depth: usize,
) -> Option<Rejection>
where
    F: Fn(&str, &str) -> Option<Rejection>,
{
    if depth > MAX_DEPTH {
        // Fail closed: a deep structure we cannot fully scan is rejected,
        // not silently passed.
        return Some(Rejection {
            location: location.to_string(),
            category: "max_depth_exceeded",
        });
    }
    match value {
        // A metadata leaf over the scanner's window cannot be scanned as
        // one string; say so distinctly instead of reporting a generic
        // oversized input, so the caller can fix the field it names.
        Value::String(s) if s.len() > MAX_SCAN_BYTES => Some(Rejection {
            location: location.to_string(),
            category: "oversized_metadata",
        }),
        Value::String(s) => scan_text(s, location),
        Value::Array(arr) => {
            for (i, item) in arr.iter().enumerate() {
                if let Some(r) =
                    scan_value_with(scan_text, item, &format!("{}.{}", location, i), depth + 1)
                {
                    return Some(r);
                }
            }
            None
        }
        Value::Object(map) => {
            for (key, val) in map {
                let loc = if location.is_empty() {
                    key.clone()
                } else {
                    format!("{}.{}", location, key)
                };
                // A key is user-controlled too: `{"ghp_…": "x"}` is a
                // credential the value scan would never see. The location
                // names the field's position without echoing the key: a
                // rejection is logged and returned to the caller, so it
                // must not repeat the credential.
                let key_loc = format!("{location}.<key>");
                if let Some(r) = scan_text(key, &key_loc) {
                    return Some(r);
                }
                if let Some(r) = scan_value_with(scan_text, val, &loc, depth + 1) {
                    return Some(r);
                }
            }
            None
        }
        // numbers, bools, null: not secrets
        _ => None,
    }
}

/// Whether a candidate value token looks like a real high-entropy secret:
/// length >= 24 chars, mixed character classes (digits combined with
/// uppercase/lowercase letters), and Shannon entropy >= 4.2 bits/char.
fn meets_high_entropy(token: &str) -> bool {
    if token.chars().count() < 24 {
        return false;
    }
    let mut has_digit = false;
    let mut has_letter = false;
    for c in token.chars() {
        if c.is_ascii_digit() {
            has_digit = true;
        } else if c.is_uppercase() || c.is_lowercase() {
            has_letter = true;
        }
    }
    if !(has_digit && has_letter) {
        return false;
    }
    shannon_entropy_bits_per_char(token).map_or(false, |h| h >= 4.2)
}

/// Shannon entropy of `s` in bits per character.
fn shannon_entropy_bits_per_char(s: &str) -> Option<f64> {
    let len = s.chars().count();
    if len == 0 {
        return None;
    }
    let mut counts: std::collections::HashMap<char, usize> = std::collections::HashMap::new();
    for c in s.chars() {
        *counts.entry(c).or_insert(0) += 1;
    }
    let len = len as f64;
    Some(
        counts
            .values()
            .map(|c| {
                let p = *c as f64 / len;
                -p * p.log2()
            })
            .sum(),
    )
}

/// Whether the (lowercased) text matches known placeholder/documentation
/// fixture patterns and should not be treated as a secret.
fn is_placeholder(lower: &str) -> bool {
    let trimmed = lower.trim();
    // Empty or near-empty.
    if trimmed.is_empty() {
        return true;
    }
    // Variable references and angle-bracket placeholders. Only single,
    // short, one-line placeholder tokens are excluded: a document that
    // merely begins with `<` (e.g. `<config>password = ...</config>`) or
    // `${` must still reach the real detectors, not bypass the scanner.
    if trimmed.starts_with('<')
        && trimmed.ends_with('>')
        && trimmed.len() <= 64
        && !trimmed[1..trimmed.len() - 1].contains('<')
        && !trimmed[1..trimmed.len() - 1].contains('>')
        && !trimmed.contains('\n')
    {
        return true;
    }
    if trimmed.starts_with("${")
        && trimmed.ends_with('}')
        && trimmed.len() <= 64
        && !trimmed[2..trimmed.len() - 1].contains('}')
        && !trimmed.contains('\n')
    {
        return true;
    }
    // Repeated asterisks or dots masking a value.
    if trimmed.chars().all(|c| c == '*' || c == '.') {
        return true;
    }
    // Repeated same character (e.g. "xxxx", "aaaa", "***").
    if trimmed.len() >= 3
        && trimmed
            .chars()
            .all(|c| c == trimmed.chars().next().unwrap_or('x'))
    {
        return true;
    }
    // Known placeholder strings: whole-value match for short patterns like
    // "changeme" to avoid false positives on real tokens that happen to
    // contain common substrings (e.g. "example" in AWS example keys).
    for pat in PLACEHOLDER_PATTERNS {
        if trimmed == *pat {
            return true;
        }
    }
    false
}

/// Split a JWT string into its three base64url segments.
/// Returns `None` if there aren't exactly three segments.
fn find_jwt_segments(s: &str) -> Option<(&str, &str, &str)> {
    let mut parts = s.split('.');
    let a = parts.next()?;
    let b = parts.next()?;
    let c = parts.next()?;
    if parts.next().is_some() {
        return None; // too many dots
    }
    Some((a, b, c))
}

/// Decode a base64url segment and check it's a JSON object containing
/// both `alg` and `typ` keys (case-sensitive).
fn is_valid_jwt_header(segment: &str) -> bool {
    // base64url decode: replace URL chars, add padding.
    let mut s = segment.replace('-', "+").replace('_', "/");
    while s.len() % 4 != 0 {
        s.push('=');
    }
    let decoded = match base64_decode(&s) {
        Some(d) => d,
        None => return false,
    };
    let val: Value = match serde_json::from_slice(&decoded) {
        Ok(v) => v,
        Err(_) => return false,
    };
    match &val {
        Value::Object(map) => map.contains_key("alg") && map.contains_key("typ"),
        _ => false,
    }
}

/// Minimal base64 decoder (no external crate needed; regex is the only dep).
fn base64_decode(input: &str) -> Option<Vec<u8>> {
    // Lookup table: ASCII byte -> base64 value, -1 for invalid.
    // Built at runtime (once) because const fn can't iterate in stable Rust.
    use std::sync::OnceLock;
    static TABLE: OnceLock<[i8; 128]> = OnceLock::new();
    let table = TABLE.get_or_init(|| {
        let mut t = [-1i8; 128];
        let alphabet = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let mut i = 0;
        while i < alphabet.len() {
            t[alphabet[i] as usize] = i as i8;
            i += 1;
        }
        t
    });

    let input = input.trim_end_matches('=');
    let len = input.len();
    if len == 0 {
        return Some(Vec::new());
    }

    let mut output = Vec::with_capacity(len * 3 / 4);
    let mut buf: u32 = 0;
    let mut bits: u32 = 0;

    for &byte in input.as_bytes() {
        let val = if byte < 128 { table[byte as usize] } else { -1 };
        if val < 0 {
            return None; // invalid character
        }
        buf = (buf << 6) | (val as u32);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            output.push((buf >> bits) as u8);
        }
    }

    Some(output)
}

// ─── Kani pilot: proofs over the production decision functions ─────────────
//
// Harnesses call the production functions above directly — the verified
// code is the shipped code (refactor-in-place; see the pilot's G2 bridge
// design). Properties are factored as plain functions in `mod properties`
// so the same property can be driven by Kani (proof) and by bolero under
// `cargo test` (dual-use).

#[cfg(any(test, kani))]
mod properties {
    use super::*;

    /// Reference base64 encoder (standard alphabet, with padding), used to
    /// state the decode round-trip property. Array-based: a `String::push`
    /// loop over symbolic content is needlessly expensive under CBMC.
    pub fn base64_encode_ref(data: &[u8]) -> String {
        const CHARS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let mut out = [0u8; 12];
        let mut n = 0usize;
        for chunk in data.chunks(3) {
            let b0 = chunk[0] as u32;
            let b1 = chunk.get(1).map_or(0, |&b| b as u32);
            let b2 = chunk.get(2).map_or(0, |&b| b as u32);
            let triple = (b0 << 16) | (b1 << 8) | b2;
            out[n] = CHARS[((triple >> 18) & 0x3F) as usize];
            out[n + 1] = CHARS[((triple >> 12) & 0x3F) as usize];
            out[n + 2] = if chunk.len() > 1 {
                CHARS[((triple >> 6) & 0x3F) as usize]
            } else {
                b'='
            };
            out[n + 3] = if chunk.len() > 2 {
                CHARS[(triple & 0x3F) as usize]
            } else {
                b'='
            };
            n += 4;
        }
        // All written bytes are ASCII (CHARS or '='), so this is valid UTF-8.
        unsafe { String::from_utf8_unchecked(out[..n].to_vec()) }
    }

    /// `find_jwt_segments` spec: returns Some iff the input has exactly
    /// two dots, and then the parts are exactly the dot-separated
    /// segments, in order, with no dot inside any part. Assertions are
    /// manual byte loops: `format!`/`assert_eq!` on heap values drag
    /// `core::fmt` into a Kani reachable set and price the proof out.
    pub fn prop_jwt_segments(s: &str) {
        let sb = s.as_bytes();
        let mut dots = 0usize;
        for i in 0..sb.len() {
            if sb[i] == b'.' {
                dots += 1;
            }
        }
        match find_jwt_segments(s) {
            Some((a, b, c)) => {
                assert!(dots == 2);
                let (ab, bb, cb) = (a.as_bytes(), b.as_bytes(), c.as_bytes());
                assert!(ab.len() + bb.len() + cb.len() + 2 == sb.len());
                for i in 0..ab.len() {
                    assert!(sb[i] == ab[i] && ab[i] != b'.');
                }
                assert!(sb[ab.len()] == b'.');
                for i in 0..bb.len() {
                    assert!(sb[ab.len() + 1 + i] == bb[i] && bb[i] != b'.');
                }
                assert!(sb[ab.len() + 1 + bb.len()] == b'.');
                for i in 0..cb.len() {
                    assert!(sb[ab.len() + bb.len() + 2 + i] == cb[i] && cb[i] != b'.');
                }
            }
            None => assert!(dots != 2),
        }
    }

    /// `base64_decode` round-trip: decoding the reference encoding of
    /// any byte string returns that byte string.
    pub fn prop_base64_roundtrip(data: &[u8]) {
        let enc = base64_encode_ref(data);
        assert!(base64_decode(&enc) == Some(data.to_vec()));
    }

    /// `base64_decode` rejection: any character outside the alphabet
    /// (after trailing `=` padding is trimmed) makes the input invalid,
    /// and decoding never panics either way.
    pub fn prop_base64_invalid_char(s: &str) {
        let trimmed = s.trim_end_matches('=');
        let mut has_invalid = false;
        for i in 0..trimmed.len() {
            let b = trimmed.as_bytes()[i];
            if !(b.is_ascii_alphanumeric() || b == b'+' || b == b'/') {
                has_invalid = true;
            }
        }
        let r = base64_decode(s);
        if has_invalid {
            assert!(r.is_none());
        }
    }
}

#[cfg(kani)]
mod kani_proofs {
    use super::properties::*;
    use super::*;

    /// A fixed-size symbolic string over a concrete ASCII alphabet.
    ///
    /// Manual membership loop, not `slice::contains`: the pilot's rung 1
    /// showed that `contains` lowers to memchr and explodes the formula.
    fn any_alpha<const N: usize>(alphabet: &[u8]) -> [u8; N] {
        let mut buf = [0u8; N];
        for slot in buf.iter_mut() {
            let c: u8 = kani::any();
            let mut ok = false;
            for &a in alphabet {
                if c == a {
                    ok = true;
                }
            }
            kani::assume(ok);
            *slot = c;
        }
        buf
    }

    /// Tier 1: `is_placeholder` decision battery (the M5 property).
    /// The symbolic form of this harness timed out at 600 s at both
    /// N=16 and N=8 (Stage 1 log, rung 1): the function's composed
    /// string searches price out a symbolic proof on this machine.
    /// The concrete battery is the verified form; the general property
    /// is sampled by the bolero drivers under `cargo test`.
    #[kani::proof]
    #[kani::unwind(64)]
    fn tier1_placeholder_battery() {
        assert!(is_placeholder("changeme"));
        assert!(is_placeholder(" your-api-key-here "));
        assert!(is_placeholder("<api-key>"));
        assert!(is_placeholder("${API_KEY}"));
        assert!(is_placeholder("***"));
        assert!(is_placeholder("xxxx"));
        assert!(!is_placeholder("xchangeme"));
        assert!(!is_placeholder("hunter2"));
        assert!(!is_placeholder("correct-horse-battery"));
    }

    /// Tier 1: exemption predicates, split per predicate (the combined
    /// battery exhausted CBMC's memory on this machine; Stage 1 log).
    /// The predicates must reject password-shaped counterexamples (a
    /// value that merely contains a dot is not a member chain, a plain
    /// word is not a type name) while accepting the code shapes they
    /// exist for.
    /// Tier 1: `looks_like_member_chain` battery. PRODUCED BUT NOT
    /// VERIFIED: this harness exhausted CBMC's memory on this machine
    /// in three forms (8 assertions in one harness; two 4-assertion
    /// splits) — the `str::split` iterator machinery in the callee is
    /// the binding constraint (Stage 1 log). Kept as the produced
    /// artifact; the property is carried by the native test
    /// `exemption_predicates_cases` under `cargo test`, and the CI
    /// lane enumerates only the verified harnesses.
    #[kani::proof]
    #[kani::unwind(64)]
    fn tier1_exempt_member_chain() {
        assert!(looks_like_member_chain("self.token"));
        assert!(looks_like_member_chain("response.token"));
        assert!(looks_like_member_chain("process.env.SECRET"));
        assert!(!looks_like_member_chain("p@ss.w0rd"));
        assert!(!looks_like_member_chain("a..b"));
        assert!(!looks_like_member_chain("a."));
        assert!(!looks_like_member_chain("123.456"));
        assert!(!looks_like_member_chain("nodots"));
    }

    /// Tier 1: `looks_like_code_expression` battery.
    #[kani::proof]
    #[kani::unwind(64)]
    fn tier1_exempt_code_expression() {
        assert!(looks_like_code_expression("a::b"));
        assert!(looks_like_code_expression("x->y"));
        assert!(looks_like_code_expression("f(x)"));
        assert!(looks_like_code_expression("self.token"));
        assert!(!looks_like_code_expression("correct.horse"));
        assert!(!looks_like_code_expression("hunter2"));
    }

    /// Tier 1: `looks_like_source_reference` battery.
    #[kani::proof]
    #[kani::unwind(64)]
    fn tier1_exempt_source_reference() {
        assert!(looks_like_source_reference("response.token"));
        assert!(!looks_like_source_reference("eyJhbGciOiJIUzI1NiJ9"));
        assert!(looks_like_source_reference("a[0"));
        assert!(!looks_like_source_reference("a[0]"));
    }

    /// Tier 1: type-name / type-annotation battery.
    #[kani::proof]
    #[kani::unwind(64)]
    fn tier1_exempt_type_names() {
        assert!(is_known_type_name("String"));
        assert!(is_known_type_name("Vec<u8>"));
        assert!(is_known_type_name("Option"));
        assert!(!is_known_type_name("Stringy"));
        assert!(!is_known_type_name("MyType"));
        assert!(looks_like_type_annotation("Vec<u8>"));
        assert!(!looks_like_type_annotation(""));
    }

    /// Tier 1: `find_jwt_segments` (the M2 property), acceptance cases
    /// by direct tuple equality. PRODUCED BUT NOT VERIFIED: every
    /// attempted form exhausted CBMC's memory or timed out on this
    /// machine (Stage 1 log, rung 2a) — symbolic strings, exhaustive
    /// enumeration over {a, b, .} (length <= 5, then <= 3), a
    /// designed-family form through the shared property function, and
    /// this micro-battery: the callee's `str::split` machinery is the
    /// binding constraint. Kept as the produced artifact; the property
    /// is carried by the bolero driver and the native test
    /// `jwt_segments_cases`, and the CI lane enumerates only the
    /// verified harnesses.
    #[kani::proof]
    #[kani::unwind(64)]
    fn tier1_jwt_segments_some() {
        assert!(find_jwt_segments("a.b.c") == Some(("a", "b", "c")));
        assert!(find_jwt_segments("ab.cd.ef") == Some(("ab", "cd", "ef")));
        assert!(find_jwt_segments("..") == Some(("", "", "")));
        assert!(find_jwt_segments("a..c") == Some(("a", "", "c")));
    }

    /// Tier 1: `find_jwt_segments` rejection cases (wrong dot count).
    /// PRODUCED BUT NOT VERIFIED (same outcome as its companion;
    /// see `tier1_jwt_segments_some`).
    #[kani::proof]
    #[kani::unwind(64)]
    fn tier1_jwt_segments_none() {
        assert!(find_jwt_segments("a.b") == None);
        assert!(find_jwt_segments("a.b.c.d") == None);
        assert!(find_jwt_segments("...") == None);
        assert!(find_jwt_segments("") == None);
    }

    /// Tier 1: `base64_decode` concrete battery — pins the decode
    /// table's observable behaviour, including the OnceLock
    /// initialization path (register T1). The symbolic round-trip and
    /// invalid-char forms timed out at 600 s in three variants
    /// (Stage 1 log, rung 2b); those properties are carried by the
    /// bolero drivers under `cargo test`.
    #[kani::proof]
    #[kani::unwind(96)]
    fn tier1_base64_battery() {
        assert!(base64_decode("") == Some(Vec::new()));
        assert!(base64_decode("QQ==") == Some(b"A".to_vec()));
        assert!(base64_decode("QUI=") == Some(b"AB".to_vec()));
        assert!(base64_decode("QUJD") == Some(b"ABC".to_vec()));
        assert!(base64_decode("aGVsbG8=") == Some(b"hello".to_vec()));
        // Padding is optional: trailing '=' is trimmed before decoding.
        assert!(base64_decode("aGVsbG8") == Some(b"hello".to_vec()));
        assert!(base64_decode("====") == Some(Vec::new()));
        assert!(base64_decode("!!").is_none());
        assert!(base64_decode("aGVsbG8!").is_none());
    }

    /// Tier 1: `is_placeholder` panic-freedom over a symbolic alphabet.
    /// The concrete placeholder specification is pinned in the battery
    /// above; this harness asks whether the predicate can panic on any
    /// bounded string in its documented input class.
    #[kani::proof]
    #[kani::unwind(64)]
    fn tier1_placeholder_symbolic() {
        assert!(is_placeholder("changeme"));
        let bytes: [u8; 8] = any_alpha::<8>(b"abcdefghijklmnopqrstuvwxyz-_./:<>{}$");
        // SAFETY: the alphabet is ASCII.
        let s = unsafe { std::str::from_utf8_unchecked(&bytes) };
        let _ = is_placeholder(s);
    }

    /// Tier 1: the JWT segment specification for every string over
    /// `{a,b,.}` of length 4: `Some` iff exactly two dots, the three
    /// parts concatenate to the input, and no part contains a dot.
    /// PRODUCED BUT NOT VERIFIED: the split-iterator formula exceeds
    /// 600 s even at N=4 on this host; the panic-freedom form below is
    /// also produced-not-verified.
    #[kani::proof]
    #[kani::unwind(16)]
    fn tier1_jwt_segments_symbolic() {
        let bytes: [u8; 4] = any_alpha::<4>(b"ab.");
        // SAFETY: the alphabet is ASCII.
        let s = unsafe { std::str::from_utf8_unchecked(&bytes) };
        prop_jwt_segments(s);
    }

    /// Tier 1: `find_jwt_segments` panic-freedom for every string over
    /// `{a,b,.}` of length 4. PRODUCED BUT NOT VERIFIED: even this form
    /// exceeds 300 s on this host (the split-iterator formula, not the
    /// input count). It is retained as the artifact for the next Kani
    /// version; the escaped UTF-8 panic is carried by the native test
    /// `detect_jwt_with_multibyte_utf8_and_non_jwt_eyj_triplet_does_not_panic`.
    #[kani::proof]
    #[kani::unwind(16)]
    fn tier1_jwt_segments_panic() {
        let bytes: [u8; 4] = any_alpha::<4>(b"ab.");
        // SAFETY: the alphabet is ASCII.
        let s = unsafe { std::str::from_utf8_unchecked(&bytes) };
        let _ = find_jwt_segments(s);
    }

    /// Tier 1: base64 round-trip for every fixed 6-byte array. The
    /// reference encoder is the spec; the production decoder must
    /// return the original bytes and must not panic.
    #[kani::proof]
    #[kani::unwind(96)]
    fn tier1_base64_roundtrip_symbolic() {
        let mut data = [0u8; 6];
        for b in data.iter_mut() {
            *b = kani::any();
        }
        prop_base64_roundtrip(&data);
    }

    /// Tier 1: any bounded string with an out-of-alphabet character
    /// (after padding trim) decodes to `None` and never panics.
    #[kani::proof]
    #[kani::unwind(96)]
    fn tier1_base64_invalid_symbolic() {
        let bytes: [u8; 6] =
            any_alpha::<6>(b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/=!");
        // SAFETY: the alphabet is ASCII.
        let s = unsafe { std::str::from_utf8_unchecked(&bytes) };
        prop_base64_invalid_char(s);
    }

    /// Tier 2 oracle (register T5): rejects iff the scanned text
    /// contains the planted marker, found by a manual byte scan. The
    /// Tier 2 harnesses call `scan_value_with` with this oracle, so no
    /// `SecretScanner` receiver is fabricated (v1 register T6 retired).
    fn scan_inner_oracle(text: &str, location: &str) -> Option<Rejection> {
        let bytes = text.as_bytes();
        let marker = b"KANI_MARKER";
        if bytes.len() >= marker.len() {
            let mut i = 0;
            while i + marker.len() <= bytes.len() {
                let mut j = 0;
                let mut hit = true;
                while j < marker.len() {
                    if bytes[i + j] != marker[j] {
                        hit = false;
                        break;
                    }
                    j += 1;
                }
                if hit {
                    return Some(Rejection {
                        location: location.to_string(),
                        category: "oracle_marker",
                    });
                }
                i += 1;
            }
        }
        None
    }

    /// Tier 2: depth fail-closed in the shared traversal (no map, no
    /// scan call on this path). Concrete battery for calibration.
    #[kani::proof]
    #[kani::unwind(32)]
    fn tier2_depth_fail_closed() {
        let r = scan_value_with(&scan_inner_oracle, &Value::Null, "meta", MAX_DEPTH + 1);
        assert!(matches!(r, Some(ref rej) if rej.category == "max_depth_exceeded"));
    }

    /// Tier 2: the depth guard holds for every depth in the tested
    /// range. Symbolic (`B`).
    #[kani::proof]
    #[kani::unwind(32)]
    fn tier2_depth_symbolic() {
        let depth: usize = kani::any();
        kani::assume(depth <= MAX_DEPTH + 1);
        let r = scan_value_with(&scan_inner_oracle, &Value::Null, "meta", depth);
        if depth > MAX_DEPTH {
            assert!(matches!(r, Some(ref rej) if rej.category == "max_depth_exceeded"));
        } else {
            assert!(r.is_none());
        }
    }

    /// Tier 2: the traversal scans object keys (the M3 property) —
    /// marker in the key, clean value.
    #[kani::proof]
    #[kani::unwind(32)]
    fn tier2_key_scan() {
        let mut map = serde_json::Map::new();
        map.insert(
            "KANI_MARKER".to_string(),
            Value::String("clean".to_string()),
        );
        let r = scan_value_with(&scan_inner_oracle, &Value::Object(map), "meta", 0);
        assert!(matches!(r, Some(ref rej) if rej.category == "oracle_marker"));
    }

    /// Tier 2: every string leaf reaches the scanner. The oracle rejects
    /// any non-empty text, so the assertion fails if the traversal ever
    /// drops a leaf. This is the cheap symbolic half of the M3 family;
    /// the concrete key battery covers object keys.
    fn leaf_oracle(text: &str, _location: &str) -> Option<Rejection> {
        if text.is_empty() {
            None
        } else {
            Some(Rejection {
                location: "leaf".to_string(),
                category: "leaf_seen",
            })
        }
    }

    #[kani::proof]
    #[kani::unwind(32)]
    fn tier2_string_leaf_symbolic() {
        let bytes: [u8; 4] = any_alpha::<4>(b"abc");
        // SAFETY: the alphabet is ASCII.
        let s = unsafe { std::str::from_utf8_unchecked(&bytes) };
        let value = Value::String(s.to_string());
        let r = scan_value_with(&leaf_oracle, &value, "meta", 0);
        assert!(matches!(r, Some(ref rej) if rej.category == "leaf_seen"));
    }

    /// Tier 2: a clean symbolic key is not rejected, while a marker key
    /// is. This is the symbolic form of the M3 property.
    #[kani::proof]
    #[kani::unwind(32)]
    fn tier2_key_scan_symbolic() {
        let key_bytes: [u8; 4] = any_alpha::<4>(b"abcdefghijklmnopqrstuvwxyz0123456789_-");
        // SAFETY: the alphabet is ASCII.
        let key = unsafe { std::str::from_utf8_unchecked(&key_bytes) };
        let mut map = serde_json::Map::new();
        map.insert(key.to_string(), Value::String("clean".to_string()));
        let r = scan_value_with(&scan_inner_oracle, &Value::Object(map), "meta", 0);
        assert!(r.is_none());

        let mut marker_map = serde_json::Map::new();
        marker_map.insert(
            "KANI_MARKER".to_string(),
            Value::String("clean".to_string()),
        );
        let r = scan_value_with(&scan_inner_oracle, &Value::Object(marker_map), "meta", 0);
        assert!(matches!(r, Some(ref rej) if rej.category == "oracle_marker"));
    }

    /// Entropy stub (register T3-S1): constant entropy at the threshold.
    fn entropy_at_threshold(_s: &str) -> Option<f64> {
        Some(4.2)
    }

    /// Entropy stub (register T3-S1): constant entropy below threshold.
    fn entropy_below_threshold(_s: &str) -> Option<f64> {
        Some(4.19)
    }

    /// Entropy stub (register T3-S1): constant entropy above threshold.
    fn entropy_above_threshold(_s: &str) -> Option<f64> {
        Some(4.21)
    }

    /// Symbolic entropy stub (register T3): the value is set by the
    /// harness, so one proof covers every `f64` the entropy function
    /// could return rather than three chosen constants.
    static mut KANI_ENTROPY: f64 = 0.0;

    fn entropy_symbolic(_s: &str) -> Option<f64> {
        Some(unsafe { KANI_ENTROPY })
    }

    /// M4 threshold decision for a symbolic entropy value: for a token
    /// that meets the length/class preconditions, `meets_high_entropy`
    /// must return exactly `e >= 4.2`. This is the proof form of the
    /// three-constant battery below.
    #[kani::proof]
    #[kani::stub(shannon_entropy_bits_per_char, entropy_symbolic)]
    fn tier1_entropy_symbolic() {
        let e: f64 = kani::any();
        // SAFETY: single-threaded harness; the stub reads this slot.
        unsafe {
            KANI_ENTROPY = e;
        }
        let got = meets_high_entropy("abcdefghijklmnopqrstuv1x");
        assert!(got == (e >= 4.2));
    }

    /// M4 threshold decision: with the entropy value stubbed to
    /// exactly the threshold, `meets_high_entropy` must accept (the
    /// comparison is `>=`). A `>` mutant flips this harness.
    /// Verified modulo T3 (Stage 1 log).
    #[kani::proof]
    #[kani::stub(shannon_entropy_bits_per_char, entropy_at_threshold)]
    fn tier1_entropy_at_threshold() {
        assert!(meets_high_entropy("abcdefghijklmnopqrstuv1x"));
    }

    /// M4 companion: just below the threshold, the decision is reject.
    #[kani::proof]
    #[kani::stub(shannon_entropy_bits_per_char, entropy_below_threshold)]
    fn tier1_entropy_below_threshold() {
        assert!(!meets_high_entropy("abcdefghijklmnopqrstuv1x"));
    }

    /// M4 companion: just above the threshold, the decision is accept.
    #[kani::proof]
    #[kani::stub(shannon_entropy_bits_per_char, entropy_above_threshold)]
    fn tier1_entropy_above_threshold() {
        assert!(meets_high_entropy("abcdefghijklmnopqrstuv1x"));
    }
}

/// Scan user-controlled content + metadata at an entry point.
/// Returns `Some(Rejection)` on detection, `None` if clean.
pub fn scan_entry_point(
    content: &str,
    metadata: &Value,
    entry_point_name: &str,
) -> Option<Rejection> {
    // 1. Scan content string.
    if let Some(r) = SecretScanner::scan_str(content, &format!("{}.content", entry_point_name)) {
        return Some(r);
    }
    // 2. Scan metadata recursively.
    if let Some(r) = SecretScanner::scan_value(metadata, &format!("{}.metadata", entry_point_name))
    {
        return Some(r);
    }
    None
}

// ─── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // ── Provider token forms ────────────────────────────────────────────

    #[test]
    fn github_pat_full_form_detected() {
        // ghp_ + 36 base62 = 40 chars
        let token = "ghp_ABCDEFGHIJKLMNOPQRSTUVWXYZabcdef123456";
        let r = SecretScanner::scan_str(token, "content").unwrap();
        assert_eq!(r.category, "github_pat");
    }

    #[test]
    fn github_pat_truncated_rejected() {
        // Too short
        let token = "ghp_abc";
        assert!(SecretScanner::scan_str(token, "content").is_none());
    }

    #[test]
    fn anthropic_token_detected() {
        let token = "sk-ant-api03-abcdefghijklmnopqrstuvwxyz1234567890";
        let r = SecretScanner::scan_str(token, "content").unwrap();
        assert_eq!(r.category, "anthropic_token");
    }

    #[test]
    fn anthropic_token_truncated_rejected() {
        let token = "sk-ant-abc";
        assert!(SecretScanner::scan_str(token, "content").is_none());
    }

    #[test]
    fn openai_key_detected() {
        // sk- + base62, ~51 chars total
        let token = format!(
            "sk-{}",
            "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789".repeat(1)
        );
        let r = SecretScanner::scan_str(&token, "content");
        // This is 4+62 = 66 chars, well over min_len=40
        assert!(r.is_some(), "sk- key should be detected");
    }

    #[test]
    fn openai_project_key_detected() {
        let token = format!(
            "sk-proj-{}",
            "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789".repeat(2)
        );
        let r = SecretScanner::scan_str(&token, "content").unwrap();
        assert_eq!(r.category, "openai_project_key");
    }

    #[test]
    fn slack_token_detected() {
        let token = format!("xoxb-{}", "0123456789abcdef".repeat(4));
        let r = SecretScanner::scan_str(&token, "content").unwrap();
        assert_eq!(r.category, "slack_token");
    }

    #[test]
    fn aws_access_key_detected() {
        let token = "AKIAIOSFODNN7EXAMPLE"; // official AWS example
        let r = SecretScanner::scan_str(token, "content").unwrap();
        assert_eq!(r.category, "aws_access_key");
    }

    #[test]
    fn aws_access_key_wrong_suffix_rejected() {
        let token = "AKIAIOSFODNN7"; // only 13 chars, not 20
        assert!(SecretScanner::scan_str(token, "content").is_none());
    }

    // ── Key=value variants ──────────────────────────────────────────────

    #[test]
    fn password_equals_catches_weak_password() {
        let text = "password = hunter2";
        let r = SecretScanner::scan_str(text, "content").unwrap();
        assert_eq!(r.category, "password");
    }

    #[test]
    fn password_colon_catches_value() {
        let text = "password: hunter2";
        let r = SecretScanner::scan_str(text, "content").unwrap();
        assert_eq!(r.category, "password");
    }

    #[test]
    fn json_password_key_detected() {
        let text = r#"{"password": "hunter2"}"#;
        let r = SecretScanner::scan_str(text, "content").unwrap();
        assert_eq!(r.category, "password");
    }

    #[test]
    fn api_key_with_whitespace() {
        let text = "api_key  =  ghp_ABCDEFGHIJKLMNOPQRSTUVWXYZabcdef123456";
        let r = SecretScanner::scan_str(text, "content").unwrap();
        assert!(r.category == "github_pat" || r.category == "password");
    }

    #[test]
    fn bearer_token_detected() {
        let text = "bearer = eyJhbGciOiJIUzI1NiJ9.eyJ0eXAiOiJKV1QifQ.xxx";
        let r = SecretScanner::scan_str(text, "content");
        assert!(r.is_some(), "bearer with JWT should be caught");
    }

    // ── JWT detection ───────────────────────────────────────────────────

    #[test]
    fn standard_jwt_detected() {
        // alg: HS256, typ: JWT
        let header = base64_encode(br#"{"alg":"HS256","typ":"JWT"}"#);
        let payload = base64_encode(br#"{"sub":"1234567890","name":"John"}"#);
        let sig = "SflKxwRJSMeKKF2QT4fwpMeJf36POk6yJV_adQssw5c";
        let token = format!("{}.{}.{}", header, payload, sig);
        let r = SecretScanner::scan_str(&token, "content").unwrap();
        assert_eq!(r.category, "jwt");
    }

    #[test]
    fn jwt_with_reordered_header_detected() {
        // typ before alg — should still match
        let header = base64_encode(br#"{"typ":"JWT","alg":"RS256"}"#);
        let payload = base64_encode(br#"{"data":"test"}"#);
        let sig = "signature";
        let token = format!("{}.{}.{}", header, payload, sig);
        let r = SecretScanner::scan_str(&token, "content").unwrap();
        assert_eq!(r.category, "jwt");
    }

    #[test]
    fn jwt_without_alg_or_typ_not_detected() {
        // Missing `typ` — not a JWT per spec
        let header = base64_encode(br#"{"alg":"HS256"}"#);
        let payload = base64_encode(br#"{"data":"test"}"#);
        let sig = "signature";
        let token = format!("{}.{}.{}", header, payload, sig);
        // Should NOT be detected as JWT (no typ)
        // It may still be caught as provider token if it looks like one,
        // but the JWT path should not fire.
        let scanner = SecretScanner::build().unwrap();
        assert!(scanner.detect_jwt(&token).is_none());
    }

    #[test]
    fn truncated_jwt_not_detected() {
        // Only two segments
        let header = base64_encode(br#"{"alg":"HS256","typ":"JWT"}"#);
        let payload = base64_encode(br#"{"data":"test"}"#);
        let token = format!("{}.{}", header, payload);
        let scanner = SecretScanner::build().unwrap();
        assert!(scanner.detect_jwt(&token).is_none());
    }

    // ── Placeholders / false positives ──────────────────────────────────

    #[test]
    fn placeholder_not_detected() {
        assert!(SecretScanner::scan_str("${API_KEY}", "content").is_none());
    }

    #[test]
    fn angle_bracket_placeholder_not_detected() {
        assert!(SecretScanner::scan_str("<your-api-key-here>", "content").is_none());
    }

    #[test]
    fn asterisks_not_detected() {
        assert!(SecretScanner::scan_str("password: ***", "content").is_none());
    }

    #[test]
    fn changeme_not_detected() {
        assert!(SecretScanner::scan_str("password = changeme", "content").is_none());
    }

    #[test]
    fn repeated_chars_not_detected() {
        assert!(SecretScanner::scan_str("xxx", "content").is_none());
        assert!(SecretScanner::scan_str("aaaa", "content").is_none());
    }

    #[test]
    fn empty_value_not_detected() {
        assert!(SecretScanner::scan_str("password =", "content").is_none());
    }

    // ── Weak password caught regardless of entropy ──────────────────────

    #[test]
    fn low_entropy_password_next_to_keyword_caught() {
        let text = "password = 1234";
        let r = SecretScanner::scan_str(text, "content").unwrap();
        assert_eq!(r.category, "password");
    }

    #[test]
    fn single_char_password_not_caught() {
        // Single char is below minimum length of 4
        assert!(SecretScanner::scan_str("password = a", "content").is_none());
    }

    // ── Benign high-entropy content (git SHA hashes) ────────────────────

    #[test]
    fn git_sha_hash_in_prose_not_detected() {
        let text = "The commit abcdef1234567890abcdef1234567890abcdef12 fixed the issue.";
        assert!(SecretScanner::scan_str(text, "content").is_none());
    }

    // ── High-entropy assignment-style secrets ───────────────────────────

    #[test]
    fn high_entropy_assignment_detected() {
        // 30 unique mixed-class chars: ~4.9 bits/char.
        let token = "aB3xZ9kQ7wE2rT8uI1oP6pM4nC5vJ0";
        let text = format!("api_key = {}", token);
        let r = SecretScanner::scan_str(&text, "content").unwrap();
        assert_eq!(r.category, "high_entropy_secret");
    }

    #[test]
    fn high_entropy_low_entropy_value_falls_back_to_password() {
        // >=24 chars but (almost) all the same character → entropy below 4.2.
        let r =
            SecretScanner::scan_str("api_key = aaaaaaaaaaaaaaaaaaaaaaaaaab1", "content").unwrap();
        assert_eq!(r.category, "password");
    }

    #[test]
    fn short_value_not_high_entropy_detected() {
        // 20 chars, below the 24-char threshold.
        assert!(
            SecretScanner::scan_str("api_key = aB3xZ9kQ7wE2rT8uI1oP", "content").is_none()
                || SecretScanner::scan_str("api_key = aB3xZ9kQ7wE2rT8uI1oP", "content")
                    .map(|r| r.category == "password")
                    .unwrap_or(false)
        );
    }

    #[test]
    fn quoted_high_entropy_value_detected() {
        let token = "aB3xZ9kQ7wE2rT8uI1oP6pM4nC5vJ0";
        let text = format!(r#"{{"client_secret": "{}"}}"#, token);
        let r = SecretScanner::scan_str(&text, "content").unwrap();
        assert_eq!(r.category, "high_entropy_secret");
    }

    // ── Metadata scanning ───────────────────────────────────────────────

    #[test]
    fn nested_metadata_secret_detected() {
        let meta = json!({
            "refs": [
                { "file": "src/main.rs" },
                { "secret": "ghp_ABCDEFGHIJKLMNOPQRSTUVWXYZabcdef123456" }
            ]
        });
        let r = SecretScanner::scan_value(&meta, "metadata").unwrap();
        assert_eq!(r.category, "github_pat");
    }

    #[test]
    fn metadata_array_leaf_detected() {
        let meta = json!({
            "tags": ["safe", "password = hunter2"]
        });
        let r = SecretScanner::scan_value(&meta, "metadata").unwrap();
        assert_eq!(r.category, "password");
    }

    /// A credential can be the key, not only a value.
    #[test]
    fn metadata_key_carrying_a_secret_is_detected() {
        let mut map = serde_json::Map::new();
        map.insert(
            "ghp_ABCDEFGHIJKLMNOPQRSTUVWXYZabcdef123456".to_string(),
            json!("benign"),
        );
        let r = SecretScanner::scan_value(&Value::Object(map), "metadata").unwrap();
        assert_eq!(r.category, "github_pat");
        assert!(
            r.location.contains("<key>"),
            "the location names the key: {}",
            r.location
        );
        assert!(
            !r.to_string().contains("ghp_"),
            "the rejection must not echo the credential-bearing key: {r}"
        );
    }

    /// A metadata leaf over the scan window has its own reason, so the caller
    /// can fix the named field instead of seeing a generic oversized input.
    #[test]
    fn oversized_metadata_leaf_has_a_distinct_reason() {
        let meta = json!({ "note": "a".repeat(MAX_SCAN_BYTES + 1) });
        let r = SecretScanner::scan_value(&meta, "metadata").unwrap();
        assert_eq!(r.category, "oversized_metadata");
        assert_eq!(r.location, "metadata.note");
    }

    /// A long single line has no newline to chunk on; the overlap must still
    /// contain a token that straddles the scan boundary whole. A JWT needs all
    /// three segments, so a split at the boundary cannot be rescued by any
    /// other detector.
    #[test]
    fn single_line_token_straddling_a_chunk_boundary_is_detected() {
        let header = base64_encode(br#"{"alg":"HS256","typ":"JWT"}"#);
        let payload = base64_encode(br#"{"sub":"1234567890","name":"John"}"#);
        let jwt = format!(
            "{}.{}.SflKxwRJSMeKKF2QT4fwpMeJf36POk6yJV_adQssw5c",
            header, payload
        );
        let mut text = "a".repeat(MAX_SCAN_BYTES - 10);
        text.push_str(&jwt);
        assert!(
            text.len() > MAX_SCAN_BYTES,
            "the fixture crosses the chunk window"
        );
        let r = SecretScanner::scan_str(&text, "content").unwrap();
        assert_eq!(r.category, "jwt");
    }

    #[test]
    fn overly_deep_metadata_rejected() {
        let mut v = json!({ "leaf": "fine" });
        for _ in 0..12 {
            v = json!({ "nested": v });
        }
        let r = SecretScanner::scan_value(&v, "metadata");
        assert_eq!(r.map(|r| r.category), Some("max_depth_exceeded"));
    }

    // ── Scan entry point integration ────────────────────────────────────

    #[test]
    fn entry_point_scans_content_and_metadata() {
        // Clean content, dirty metadata
        let meta = json!({ "token": "ghp_ABCDEFGHIJKLMNOPQRSTUVWXYZabcdef123456" });
        let r = scan_entry_point("all good", &meta, "add_memory").unwrap();
        assert_eq!(r.location, "add_memory.metadata.token");

        // Dirty content
        let meta2 = json!({});
        let r2 = scan_entry_point("password = hunter2", &meta2, "add_memory").unwrap();
        assert_eq!(r2.location, "add_memory.content");
    }

    // ── All three entry points same outcome ─────────────────────────────

    #[test]
    fn same_outcome_on_add_supersede_and_edit() {
        let dirty_content = "api_key = ghp_ABCDEFGHIJKLMNOPQRSTUVWXYZabcdef123456";
        let clean_meta = json!({});

        for entry in &["add_memory", "supersede_memory", "edit_memory"] {
            let r = scan_entry_point(dirty_content, &clean_meta, entry);
            assert!(r.is_some(), "{} should reject secret", entry);
        }
    }

    #[test]
    fn clean_input_passes_all_entry_points() {
        let clean = "Always use podman, not docker.";
        let meta = json!({ "governs": ["src/daemon.rs"] });

        for entry in &["add_memory", "supersede_memory", "edit_memory"] {
            assert!(
                scan_entry_point(clean, &meta, entry).is_none(),
                "{} should pass",
                entry
            );
        }
    }

    // ── Scanner bypass regression (AN-S1) ───────────────────────────────

    #[test]
    fn document_starting_with_angle_bracket_not_bypassing_scanner() {
        // A full XML/HTML/Markdown document that happens to start with `<`
        // must not disable the scanner; the embedded secret is rejected.
        // A document that starts with `${` likewise.
        let xml = r#"<config>api_key = "sk-live-1234567890abcdefghijklmnop"</config>"#;
        let r = SecretScanner::scan_str(xml, "content");
        assert!(
            r.is_some(),
            "secret inside <config> document must be rejected"
        );
        assert!(
            matches!(
                r.as_ref().map(|r| r.category),
                Some("high_entropy_secret") | Some("password") | Some("openai_key")
            ),
            "unexpected category: {:?}",
            r.map(|r| r.category)
        );

        // While single placeholder tokens are still allowed.
        assert!(SecretScanner::scan_str("<your-api-key>", "content").is_none());
        assert!(SecretScanner::scan_str("${API_KEY}", "content").is_none());
    }

    // ── Large inputs ────────────────────────────────────────────────────

    #[test]
    fn large_clean_text_is_scanned_not_rejected_for_size() {
        let text = "a".repeat(MAX_SCAN_BYTES + 10_000);
        assert!(
            SecretScanner::scan_str(&text, "content").is_none(),
            "a clean file larger than the chunk size must not be rejected as oversized"
        );
    }

    #[test]
    fn large_text_with_a_secret_across_a_chunk_boundary_is_detected() {
        let mut text = "a".repeat(MAX_SCAN_BYTES - 10);
        // Built at runtime so no secret-shaped literal sits in the source.
        text.push_str(&format!(" ghp_{}", "Ab1".repeat(12)));
        text.push_str(&"b".repeat(10_000));
        let rejection = SecretScanner::scan_str(&text, "content").expect("secret detected");
        assert_eq!(rejection.category, "github_pat");
    }

    #[test]
    fn source_assignments_from_functions_are_not_secrets() {
        let text = "let token = read_token();\n\
                    let password = generate_password();\n\
                    let key = crate::daemon::read_token_file(path).await?;";
        assert!(
            SecretScanner::scan_str(text, "content").is_none(),
            "source expressions are not credential literals"
        );
    }

    #[test]
    fn source_type_annotations_are_not_probed_as_passwords() {
        let text = "struct Config {\n\
                    api_key: String,\n\
                    token: String,\n\
                    path: Option<PathBuf>,\n\
                    secret: &'a str,\n\
                    bytes: [u8; 32],\n\
                    }";
        assert!(
            SecretScanner::scan_str(text, "content").is_none(),
            "type annotations are not credential values"
        );
    }

    #[test]
    fn quoted_password_literal_is_still_detected() {
        let text = "let password = \"s3cr3t-value\";";
        let rejection = SecretScanner::scan_str(text, "content").expect("quoted literal");
        assert_eq!(rejection.category, "password");
    }

    #[test]
    fn long_jwt_across_a_line_boundary_is_detected() {
        let header = base64_encode(br#"{"alg":"HS256","typ":"JWT"}"#);
        // The payload is far longer than the old 4 KB overlap; a line-aware
        // chunk boundary must keep the whole token in one scan.
        let payload = format!("eyJ{}", "A".repeat(200_000));
        let sig = "B".repeat(200);
        let jwt = format!("{header}.{payload}.{sig}");
        let mut text = "x".repeat(MAX_SCAN_BYTES - 100);
        text.push('\n');
        text.push_str(&jwt);
        text.push('\n');
        text.push_str(&"y".repeat(1_000));
        let rejection = SecretScanner::scan_str(&text, "content").expect("long JWT detected");
        assert_eq!(rejection.category, "jwt");
    }

    #[test]
    fn plaintext_config_values_with_punctuation_are_still_detected() {
        for text in [
            "password=hunter2!",
            "password = \"hunter2!;\"",
            "api_key=abc(def",
            "password=&hunter2!",
            "password=[hunter2]",
            "password: &hunter2!",
        ] {
            let rejection = SecretScanner::scan_str(text, "content")
                .unwrap_or_else(|| panic!("plaintext credential must be detected: {text}"));
            assert!(
                rejection.category == "password" || rejection.category == "high_entropy_secret",
                "unexpected category for {text}: {}",
                rejection.category
            );
        }
    }

    #[test]
    fn multiline_assignment_across_a_chunk_boundary_is_detected() {
        // The first chunk ends on the newline after `password =`; the value is
        // on the next line and must still be associated with its key.
        let mut text = "x".repeat(MAX_SCAN_BYTES - 100);
        text.push_str("let password =\n");
        text.push_str(&" ".repeat(200));
        text.push_str("\"hunter2\";\n");
        text.push_str(&" ".repeat(1_000));
        let rejection =
            SecretScanner::scan_str(&text, "content").expect("multiline assignment detected");
        assert_eq!(rejection.category, "password");
    }

    #[test]
    fn secret_source_expression_matrix() {
        for clean in [
            "let token = self.token;",
            "token=None",
            "const secret = process.env.SECRET;",
            "// token = None",
            "let value = parts[0];",
            "let token = response.token;",
            "struct Cfg { api_key: &self.api_key }",
            "    token: normalize_query_token(&stem),",
            "    secret: process.env.SECRET,",
        ] {
            assert!(
                SecretScanner::scan_str(clean, "content").is_none(),
                "clean source shape was rejected: {clean}"
            );
        }
        for literal in [
            "let password = \"hunter2!\";",
            "// password = \"hunter2!\"",
            "PASSWORD=hunter2!",
            "password=correct.horse",
            "PASSWORD=hunter2[abc",
            "// password=correct.horse",
            "# password=hunter2[abc",
        ] {
            assert!(
                SecretScanner::scan_str(literal, "content").is_some(),
                "credential literal was accepted: {literal}"
            );
        }
    }

    #[test]
    fn placeholder_wrapped_token_rejected() {
        let token = format!("ghp_{}", "Ab1".repeat(12));
        for wrapped in [format!("<{token}>"), format!("${{{token}}}")] {
            let rejection = SecretScanner::scan_str(&wrapped, "content")
                .unwrap_or_else(|| panic!("wrapped token accepted: {wrapped}"));
            assert_eq!(rejection.category, "github_pat");
        }
    }

    #[test]
    fn detect_jwt_with_multibyte_utf8_and_non_jwt_eyj_triplet_does_not_panic() {
        // Reproduces the exact char-boundary panic when `jwt_re` matches a
        // non-JWT `eyJ...eyJ...` triplet after multi-byte UTF-8 box-drawing
        // characters (`─`, 3 bytes each), followed by a valid JWT.
        let prefix = "─── ".repeat(10);
        let non_jwt_triplet = "eyJnotjson.eyJpayload.sig123";
        let header = base64_encode(br#"{"alg":"HS256","typ":"JWT"}"#);
        let payload = base64_encode(br#"{"sub":"123"}"#);
        let valid_jwt = format!("{header}.{payload}.sig456");

        let clean_doc = format!("{prefix}\n{non_jwt_triplet}\n");
        assert!(SecretScanner::scan_str(&clean_doc, "content").is_none());

        let dirty_doc = format!("{prefix}\n{non_jwt_triplet}\n{valid_jwt}\n");
        let r = SecretScanner::scan_str(&dirty_doc, "content").expect("valid jwt after non-jwt");
        assert_eq!(r.category, "jwt");
    }

    // ── Kani pilot: property drivers (dual-use) ─────────────────────
    //
    // The shared property functions in `mod properties` are proven by
    // the Kani harnesses in `mod kani_proofs` where the machine allows
    // and sampled here under bolero where it does not (symbolic forms
    // that timed out; see the Stage 1 log for the produced/verified
    // split). In test mode bolero runs each driver over its built-in
    // corpus of generated inputs.

    #[test]
    fn jwt_segments_property_sampled() {
        bolero::check!()
            .with_type::<Vec<u8>>()
            .for_each(|bytes: &Vec<u8>| {
                // Map arbitrary bytes onto the structural alphabet so
                // dot placement is densely sampled.
                let s: String = bytes
                    .iter()
                    .take(12)
                    .map(|b| [b'a', b'b', b'.'][(b % 3) as usize] as char)
                    .collect();
                crate::secrets::properties::prop_jwt_segments(&s);
            });
    }

    #[test]
    fn base64_roundtrip_property_sampled() {
        bolero::check!()
            .with_type::<Vec<u8>>()
            .for_each(|bytes: &Vec<u8>| {
                // The reference encoder is sized for <= 9 input bytes.
                let n = bytes.len().min(9);
                crate::secrets::properties::prop_base64_roundtrip(&bytes[..n]);
            });
    }

    #[test]
    fn base64_invalid_char_property_sampled() {
        bolero::check!()
            .with_type::<String>()
            .for_each(|s: &String| {
                let t: String = s.chars().take(12).collect();
                crate::secrets::properties::prop_base64_invalid_char(&t);
            });
    }

    /// M1 driver: `validate_provider_form` length boundaries, called
    /// directly. The end-to-end tests above exercise the same forms
    /// through the regex phases; this test pins the boundary itself
    /// (a candidate of exactly `min_len` is accepted, one byte shorter
    /// is not) for every provider form, including the AKIA suffix rule.
    #[test]
    fn provider_form_length_boundaries() {
        let scanner = SecretScanner::instance();
        let at = |prefix: &str, total: usize| -> String {
            let mut s = String::from(prefix);
            while s.len() < total {
                s.push('A');
            }
            s
        };
        // (prefix, min_len, expected category)
        let forms = [
            ("ghp_", 16, "github_pat"),
            ("gho_", 16, "github_pat"),
            ("github_pat_", 40, "github_pat"),
            ("sk-ant-", 16, "anthropic_token"),
            ("sk-proj-", 20, "openai_project_key"),
            ("sk-", 40, "openai_key"),
            ("xoxb-", 16, "slack_token"),
        ];
        for (prefix, min_len, category) in forms {
            assert_eq!(
                scanner.validate_provider_form(&at(prefix, min_len)),
                Some(category),
                "{prefix} at min_len {min_len} must be accepted"
            );
            assert_eq!(
                scanner.validate_provider_form(&at(prefix, min_len - 1)),
                None,
                "{prefix} below min_len must be rejected"
            );
        }
        // AKIA: 4-char prefix + 16-char [0-9A-Z] suffix = 20.
        assert_eq!(
            scanner.validate_provider_form("AKIAIOSFODNN7EXAMPLE"),
            Some("aws_access_key")
        );
        // Lowercase suffix fails the suffix regex.
        assert_eq!(scanner.validate_provider_form("AKIAiosfodnn7example"), None);
        // 15-char suffix: below min_len and fails the suffix shape.
        assert_eq!(scanner.validate_provider_form("AKIAIOSFODNN7EXAMP"), None);
    }

    /// Native carrier for the exemption-predicate batteries whose
    /// Kani forms are split-limited (member chain) or verified
    /// (code expression, source reference, type names): the same
    /// assertions, executed natively.
    #[test]
    fn exemption_predicates_cases() {
        assert!(looks_like_member_chain("self.token"));
        assert!(looks_like_member_chain("response.token"));
        assert!(looks_like_member_chain("process.env.SECRET"));
        assert!(!looks_like_member_chain("p@ss.w0rd"));
        assert!(!looks_like_member_chain("a..b"));
        assert!(!looks_like_member_chain("a."));
        assert!(!looks_like_member_chain("123.456"));
        assert!(!looks_like_member_chain("nodots"));
        assert!(looks_like_code_expression("a::b"));
        assert!(looks_like_code_expression("x->y"));
        assert!(looks_like_code_expression("f(x)"));
        assert!(looks_like_code_expression("self.token"));
        assert!(!looks_like_code_expression("correct.horse"));
        assert!(!looks_like_code_expression("hunter2"));
        assert!(looks_like_source_reference("response.token"));
        assert!(!looks_like_source_reference("eyJhbGciOiJIUzI1NiJ9"));
        assert!(looks_like_source_reference("a[0"));
        assert!(!looks_like_source_reference("a[0]"));
        assert!(is_known_type_name("String"));
        assert!(is_known_type_name("Vec<u8>"));
        assert!(is_known_type_name("Option"));
        assert!(!is_known_type_name("Stringy"));
        assert!(!is_known_type_name("MyType"));
        assert!(looks_like_type_annotation("Vec<u8>"));
        assert!(!looks_like_type_annotation(""));
    }

    /// Native carrier for the `find_jwt_segments` micro-battery (the
    /// M2 property; its Kani forms are split-limited — see
    /// `tier1_jwt_segments_some`). The general property is sampled by
    /// `jwt_segments_property_sampled` above.
    #[test]
    fn jwt_segments_cases() {
        assert_eq!(find_jwt_segments("a.b.c"), Some(("a", "b", "c")));
        assert_eq!(find_jwt_segments("ab.cd.ef"), Some(("ab", "cd", "ef")));
        assert_eq!(find_jwt_segments(".."), Some(("", "", "")));
        assert_eq!(find_jwt_segments("a..c"), Some(("a", "", "c")));
        assert_eq!(find_jwt_segments(".b.c"), Some(("", "b", "c")));
        assert_eq!(find_jwt_segments("a.b."), Some(("a", "b", "")));
        assert_eq!(find_jwt_segments("a.b"), None);
        assert_eq!(find_jwt_segments("a.b.c.d"), None);
        assert_eq!(find_jwt_segments("..."), None);
        assert_eq!(find_jwt_segments(""), None);
        assert_eq!(find_jwt_segments("abc"), None);
    }

    /// JWT header validation on the canonical example header.
    #[test]
    fn jwt_header_validation_cases() {
        // {"alg":"HS256","typ":"JWT"} in base64url, unpadded.
        assert!(is_valid_jwt_header("eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9"));
        // Decodes to "hello": not JSON, not a JWT header.
        assert!(!is_valid_jwt_header("aGVsbG8"));
        // Not base64 at all.
        assert!(!is_valid_jwt_header("!!!"));
    }

    // ── Helpers ─────────────────────────────────────────────────────────

    /// Base64url-encode for test JWT headers (JWT uses base64url, not standard base64).
    fn base64_encode(input: &[u8]) -> String {
        const CHARS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
        let mut out = String::with_capacity((input.len() + 2) / 3 * 4);
        for chunk in input.chunks(3) {
            let b0 = chunk[0] as u32;
            let b1 = chunk.get(1).map_or(0, |&b| b as u32);
            let b2 = chunk.get(2).map_or(0, |&b| b as u32);
            let triple = (b0 << 16) | (b1 << 8) | b2;
            out.push(CHARS[((triple >> 18) & 0x3F) as usize] as char);
            out.push(CHARS[((triple >> 12) & 0x3F) as usize] as char);
            if chunk.len() > 1 {
                out.push(CHARS[((triple >> 6) & 0x3F) as usize] as char);
            } else {
                // base64url: no padding in JWTs
            }
            if chunk.len() > 2 {
                out.push(CHARS[(triple & 0x3F) as usize] as char);
            }
        }
        out
    }
}
