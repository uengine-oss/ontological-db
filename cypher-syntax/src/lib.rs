//! The engine's Cypher lexer, parser and AST as a plain library.
//!
//! The three files are the engine's own (`engine/src/cypher/{lexer,parser,ast}.rs`),
//! included by path: one parser, read by two programs. They use nothing but
//! each other, so they build here without pgrx or PostgreSQL. The engine is not
//! changed and does not depend on this crate.
//!
//! The Bolt gateway's AGE backend (`bolt/src/age/`) uses it to learn what the
//! engine's `og_cypher_columns()` and `og_cypher_check()` answer — the RETURN
//! column names and whether a statement writes — and to re-print a query as
//! the Cypher Apache AGE accepts.

/// The engine's bound for an open-ended variable-length relationship (`*`,
/// `*2..`): the parser fills it in. Kept equal to
/// `engine/src/cypher/mod.rs`'s `MAX_VAR_LENGTH`; a unit test reads that file
/// to hold the two together.
pub const MAX_VAR_LENGTH: u32 = 8;

#[path = "../../engine/src/cypher/ast.rs"]
pub mod ast;
#[path = "../../engine/src/cypher/lexer.rs"]
pub mod lexer;
#[path = "../../engine/src/cypher/parser.rs"]
pub mod parser;

pub use parser::parse;

#[cfg(test)]
mod same_bound {
    #[test]
    fn max_var_length_matches_the_engine() {
        let src = include_str!("../../engine/src/cypher/mod.rs");
        let want = format!("pub const MAX_VAR_LENGTH: u32 = {};", super::MAX_VAR_LENGTH);
        assert!(src.contains(&want), "engine/src/cypher/mod.rs no longer says `{want}`");
    }
}
