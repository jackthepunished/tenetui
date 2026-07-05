//! Function-level tracking (`arepo`) — parse a snapshot with tree-sitter and
//! locate function definitions by name and line range, so the player can scope
//! itself to a single function's evolution.
//!
//! Behind the `functions` feature: tree-sitter + a grammar are a heavy
//! dependency, so a default build compiles the stubs below (which find nothing)
//! and the `F` key is simply inert. Rust is the only grammar for now.

/// A function definition located in a snapshot: its name and 0-indexed,
/// inclusive line range.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FunctionDef {
    pub name: String,
    pub start_line: usize,
    pub end_line: usize,
}

/// The line range (0-indexed, inclusive) of the first function named `name` in
/// `content`, or `None` if the file type is unsupported or it isn't present.
pub fn range_of(content: &str, path: &str, name: &str) -> Option<(usize, usize)> {
    functions_in(content, path)
        .into_iter()
        .find(|f| f.name == name)
        .map(|f| (f.start_line, f.end_line))
}

#[cfg(feature = "functions")]
pub fn functions_in(content: &str, path: &str) -> Vec<FunctionDef> {
    if !path.ends_with(".rs") {
        return Vec::new();
    }
    let mut parser = tree_sitter::Parser::new();
    if parser.set_language(&tree_sitter_rust::language()).is_err() {
        return Vec::new();
    }
    let Some(tree) = parser.parse(content, None) else {
        return Vec::new();
    };

    let mut out = Vec::new();
    collect(tree.root_node(), content.as_bytes(), &mut out);
    out
}

#[cfg(feature = "functions")]
fn collect(node: tree_sitter::Node, src: &[u8], out: &mut Vec<FunctionDef>) {
    // `function_item` covers both free functions and methods in the Rust grammar.
    if node.kind() == "function_item"
        && let Some(name_node) = node.child_by_field_name("name")
        && let Ok(name) = name_node.utf8_text(src)
    {
        out.push(FunctionDef {
            name: name.to_string(),
            start_line: node.start_position().row,
            end_line: node.end_position().row,
        });
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect(child, src, out);
    }
}

#[cfg(not(feature = "functions"))]
pub fn functions_in(_content: &str, _path: &str) -> Vec<FunctionDef> {
    Vec::new()
}

#[cfg(all(test, feature = "functions"))]
mod tests {
    use super::*;

    const SRC: &str = "\
fn alpha() {
    let x = 1;
}

struct S;
impl S {
    fn beta(&self) -> usize {
        42
    }
}

fn gamma() {}
";

    #[test]
    fn finds_free_functions_and_methods_with_ranges() {
        let fns = functions_in(SRC, "x.rs");
        let names: Vec<&str> = fns.iter().map(|f| f.name.as_str()).collect();
        assert!(names.contains(&"alpha"));
        assert!(names.contains(&"beta"), "methods count: {names:?}");
        assert!(names.contains(&"gamma"));

        let alpha = fns.iter().find(|f| f.name == "alpha").unwrap();
        assert_eq!(alpha.start_line, 0); // `fn alpha() {` is line 0
        assert_eq!(alpha.end_line, 2); // closing `}` is line 2
    }

    #[test]
    fn range_of_returns_first_match_or_none() {
        assert_eq!(range_of(SRC, "x.rs", "beta"), Some((6, 8)));
        assert_eq!(range_of(SRC, "x.rs", "missing"), None);
        // Non-Rust files are unsupported.
        assert_eq!(range_of(SRC, "x.txt", "alpha"), None);
    }
}
