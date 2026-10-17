//! Validation of user-written WGSL (custom transitions and filters). The user's code sits
//! between a fixed prelude and epilogue; errors are reported against the user's own lines.

/// Assembles the full module.
pub(crate) fn assemble(prelude: &str, code: &str, epilogue: &str) -> String {
    format!("{prelude}\n{code}\n{epilogue}")
}

/// Parses and validates `code` placed between `prelude` and `epilogue`.
/// `function` must be defined; `signature` is shown when it isn't.
pub(crate) fn validate(
    prelude: &str,
    code: &str,
    epilogue: &str,
    function: &str,
    signature: &str,
) -> Result<(), String> {
    let source = assemble(prelude, code, epilogue);
    let first = prelude.lines().count() + 2;
    let user_lines = code.lines().count().max(1);
    let missing = || format!("The shader must define {signature}");
    let locate = |line: Option<u32>, message: String| -> String {
        let Some(line) = line.map(|l| l as usize) else {
            return message;
        };
        if line >= first && line < first + user_lines {
            format!("Line {}: {message}", line - first + 1)
        } else if line >= first + user_lines {
            format!("{message}\n{}", missing())
        } else {
            message
        }
    };
    let module = naga::front::wgsl::parse_str(&source).map_err(|e| {
        let line = e.location(&source).map(|l| l.line_number);
        let mut message = e.message().to_string();
        if let Some(label) = e
            .labels()
            .map(|(_, label)| label.to_string())
            .find(|l| !l.is_empty())
        {
            message = format!("{message} ({label})");
        }
        locate(line, message)
    })?;
    naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::empty(),
    )
    .validate(&module)
    .map_err(|e| {
        let line = e.location(&source).map(|l| l.line_number);
        locate(line, e.as_inner().to_string())
    })?;
    if !module
        .functions
        .iter()
        .any(|(_, f)| f.name.as_deref() == Some(function))
    {
        return Err(missing());
    }
    Ok(())
}

/// Stable hash of shader code, for pipeline caches.
pub(crate) fn hash(code: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    code.hash(&mut h);
    h.finish()
}
