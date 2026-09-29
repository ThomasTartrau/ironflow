//! Auto-registration of installed template handlers.
//!
//! Detects the `handlers()` function in the user's project and adds the
//! new handler to it. Falls back to printing the line to add manually if
//! the pattern is not recognized.
//!
//! # Examples
//!
//! ```no_run
//! use std::path::Path;
//! use ironflow_templates::auto_register::detect_and_register_handler;
//!
//! # fn example() -> Result<(), ironflow_templates::error::TemplateError> {
//! let result = detect_and_register_handler(
//!     Path::new("./src/workflows"),
//!     "ci-pipeline",
//!     "CiPipeline",
//! )?;
//! println!("{}", result.message);
//! # Ok(())
//! # }
//! ```

use std::fs;
use std::path::Path;

use crate::error::TemplateError;

/// Result of an auto-registration attempt.
#[derive(Debug)]
pub struct RegisterResult {
    /// Whether the handler was automatically registered.
    pub registered: bool,
    /// Human-readable message describing what happened.
    pub message: String,
}

/// Search for the `handlers()` function in the project and register the
/// new handler.
///
/// Looks for files named `lib.rs` or `handlers.rs` under `search_root`
/// (typically `src/workflows/` or `src/`), searching for a function
/// returning `Vec<Box<dyn WorkflowHandler>>`. If found, inserts:
/// - A `pub mod <module_name>;` declaration, public so the other crates of
///   the workspace (a runtime calling the template's webhook trigger) reach
///   its items
/// - A `pub use <module_name>::<type_name>;` re-export
/// - A `Box::new(<type_name>)` entry in the vec returned by `handlers()`
///
/// If the pattern is not found, returns a [`RegisterResult`] with
/// `registered: false` and a message showing what to add manually.
///
/// # Errors
///
/// Returns [`TemplateError::Io`] on filesystem errors.
///
/// # Examples
///
/// ```no_run
/// use std::path::Path;
/// use ironflow_templates::auto_register::detect_and_register_handler;
///
/// # fn example() -> Result<(), ironflow_templates::error::TemplateError> {
/// let result = detect_and_register_handler(
///     Path::new("./src/workflows"),
///     "hello-world",
///     "HelloWorld",
/// )?;
/// if result.registered {
///     println!("Handler registered automatically");
/// } else {
///     println!("Manual step needed: {}", result.message);
/// }
/// # Ok(())
/// # }
/// ```
pub fn detect_and_register_handler(
    search_root: &Path,
    module_name: &str,
    type_name: &str,
) -> Result<RegisterResult, TemplateError> {
    let candidates = ["lib.rs", "handlers.rs"];

    for filename in &candidates {
        let path = search_root.join(filename);
        if !path.exists() {
            continue;
        }

        let content = fs::read_to_string(&path).map_err(TemplateError::Io)?;
        if !content.contains("fn handlers()") {
            continue;
        }

        let updated = insert_handler_registration(&content, module_name, type_name);
        if let Some(new_content) = updated {
            fs::write(&path, new_content).map_err(TemplateError::Io)?;
            return Ok(RegisterResult {
                registered: true,
                message: format!("Registered {type_name} in {}", path.display()),
            });
        }
    }

    let rust_module = module_name.replace('-', "_");
    Ok(RegisterResult {
        registered: false,
        message: format!(
            "Could not detect handlers() function. Add manually:\n\
             \n\
             pub mod {rust_module};\n\
             pub use {rust_module}::{type_name};\n\
             \n\
             // In handlers():\n\
             Box::new({type_name}),",
        ),
    })
}

/// Insert mod, use, and Box::new lines into the file content.
///
/// Returns `None` if the expected patterns cannot be found.
fn insert_handler_registration(
    content: &str,
    module_name: &str,
    type_name: &str,
) -> Option<String> {
    let rust_module = module_name.replace('-', "_");

    // Skip if already registered
    if content.contains(&format!("mod {rust_module};")) {
        return None;
    }

    insert_handler_simple(content, &rust_module, type_name)
}

/// Simpler insertion strategy: find the last `mod` line, last `pub use` line,
/// and the `]` that closes the `vec![` in `handlers()`.
fn insert_handler_simple(content: &str, rust_module: &str, type_name: &str) -> Option<String> {
    let lines: Vec<&str> = content.lines().collect();

    let last_mod_idx = lines.iter().rposition(|l| {
        let t = l.trim();
        t.starts_with("mod ") && t.ends_with(';')
    })?;

    let last_use_idx = lines.iter().rposition(|l| {
        let t = l.trim();
        t.starts_with("pub use ") && t.ends_with(';')
    })?;

    let handlers_idx = lines.iter().position(|l| l.contains("fn handlers()"))?;
    let entry = format!("Box::new({type_name})");

    // `vec![...]` on one line: rewrite that line. Otherwise find the `]`
    // alone on its line that closes the vec.
    let single_line = lines[handlers_idx..]
        .iter()
        .position(|l| {
            let t = l.trim();
            t.starts_with("vec![") && (t.ends_with(']') || t.ends_with("];"))
        })
        .map(|offset| handlers_idx + offset);
    let vec_close_idx = match single_line {
        Some(idx) => idx,
        None => {
            let offset = lines[handlers_idx..].iter().rposition(|l| {
                let t = l.trim();
                t == "]" || t == "];"
            })?;
            handlers_idx + offset
        }
    };

    let mut result = Vec::with_capacity(lines.len() + 3);

    for (i, line) in lines.iter().enumerate() {
        if Some(i) == single_line {
            result.push(append_to_single_line_vec(line, &entry));
        } else {
            result.push((*line).to_string());
        }

        if i == last_mod_idx {
            result.push(format!("pub mod {rust_module};"));
        }

        if i == last_use_idx {
            result.push(format!("pub use {rust_module}::{type_name};"));
        }

        if single_line.is_none() && i == vec_close_idx - 1 {
            result.push(format!("    {entry},"));
        }
    }

    let mut output = result.join("\n");
    if content.ends_with('\n') {
        output.push('\n');
    }
    Some(output)
}

/// Append `entry` to a `vec![...]` written on one line, keeping its
/// indentation and trailing `;`.
fn append_to_single_line_vec(line: &str, entry: &str) -> String {
    let close = line.rfind(']').unwrap_or(line.len());
    let (head, tail) = line.split_at(close);
    let items = head.trim_end().trim_end_matches(',');
    if items.trim_end().ends_with("vec![") {
        format!("{items}{entry}{tail}")
    } else {
        format!("{items}, {entry}{tail}")
    }
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::*;

    fn sample_lib_rs() -> &'static str {
        "\
mod greeting;
mod deploy;

pub use greeting::Greeting;
pub use deploy::Deploy;

use ironflow_engine::handler::WorkflowHandler;

pub fn handlers() -> Vec<Box<dyn WorkflowHandler>> {
    vec![
        Box::new(Greeting),
        Box::new(Deploy),
    ]
}
"
    }

    #[test]
    fn register_in_handlers() {
        let tmp = TempDir::new().unwrap();
        let lib_path = tmp.path().join("lib.rs");
        fs::write(&lib_path, sample_lib_rs()).unwrap();

        let result = detect_and_register_handler(tmp.path(), "ci-pipeline", "CiPipeline").unwrap();

        assert!(result.registered);
        assert!(result.message.contains("CiPipeline"));

        let content = fs::read_to_string(&lib_path).unwrap();
        // Public: other crates of the workspace (the runtime) reach the
        // template's items, e.g. a webhook trigger.
        assert!(content.contains("\npub mod ci_pipeline;\n"), "{content}");
        assert!(content.contains("pub use ci_pipeline::CiPipeline;"));
        assert!(content.contains("Box::new(CiPipeline),"));
    }

    #[test]
    fn handlers_not_found() {
        let tmp = TempDir::new().unwrap();
        let lib_path = tmp.path().join("lib.rs");
        fs::write(&lib_path, "// no handlers here\n").unwrap();

        let result = detect_and_register_handler(tmp.path(), "ci-pipeline", "CiPipeline").unwrap();

        assert!(!result.registered);
        assert!(result.message.contains("Add manually"));
        assert!(result.message.contains("CiPipeline"));
        assert!(
            result.message.contains("pub mod ci_pipeline;"),
            "{}",
            result.message
        );
        assert!(result.message.contains("pub use ci_pipeline::CiPipeline;"));
    }

    #[test]
    fn no_candidate_files() {
        let tmp = TempDir::new().unwrap();
        let result = detect_and_register_handler(tmp.path(), "ci-pipeline", "CiPipeline").unwrap();

        assert!(!result.registered);
    }

    #[test]
    fn already_registered_returns_none() {
        let content = "\
mod ci_pipeline;
mod deploy;

pub use ci_pipeline::CiPipeline;
pub use deploy::Deploy;

pub fn handlers() -> Vec<Box<dyn WorkflowHandler>> {
    vec![
        Box::new(CiPipeline),
        Box::new(Deploy),
    ]
}
";
        let result = insert_handler_registration(content, "ci-pipeline", "CiPipeline");
        assert!(result.is_none());
    }

    #[test]
    fn register_in_single_line_vec() {
        let tmp = TempDir::new().unwrap();
        let lib_path = tmp.path().join("lib.rs");
        fs::write(
            &lib_path,
            "\
mod hello;

pub use hello::{Hello, HelloInput};

use ironflow_engine::handler::WorkflowHandler;

/// Every workflow handler of this project, boxed.
pub fn handlers() -> Vec<Box<dyn WorkflowHandler>> {
    vec![Box::new(Hello)]
}
",
        )
        .unwrap();

        let result =
            detect_and_register_handler(tmp.path(), "gitlab-mr-review", "GitlabMrReview").unwrap();

        assert!(result.registered, "{}", result.message);
        let content = fs::read_to_string(&lib_path).unwrap();
        assert!(content.contains("mod hello;\npub mod gitlab_mr_review;\n"));
        assert!(content.contains("pub use gitlab_mr_review::GitlabMrReview;\n"));
        assert!(
            content.contains("    vec![Box::new(Hello), Box::new(GitlabMrReview)]\n"),
            "got:\n{content}"
        );
    }

    #[test]
    fn register_in_empty_single_line_vec() {
        let tmp = TempDir::new().unwrap();
        let lib_path = tmp.path().join("lib.rs");
        fs::write(
            &lib_path,
            "\
mod hello;

pub use hello::Hello;

pub fn handlers() -> Vec<Box<dyn WorkflowHandler>> {
    vec![]
}
",
        )
        .unwrap();

        detect_and_register_handler(tmp.path(), "ci", "Ci").unwrap();

        let content = fs::read_to_string(&lib_path).unwrap();
        assert!(
            content.contains("    vec![Box::new(Ci)]\n"),
            "got:\n{content}"
        );
    }

    #[test]
    fn hyphenated_name_uses_underscore() {
        let tmp = TempDir::new().unwrap();
        let lib_path = tmp.path().join("lib.rs");
        fs::write(&lib_path, sample_lib_rs()).unwrap();

        detect_and_register_handler(tmp.path(), "my-template", "MyTemplate").unwrap();

        let content = fs::read_to_string(&lib_path).unwrap();
        assert!(content.contains("pub mod my_template;"));
        assert!(content.contains("pub use my_template::MyTemplate;"));
    }
}
