// Copyright (c) The Move Contributors
// Modifications Copyright (c) 2024 IOTA Stiftung
// SPDX-License-Identifier: Apache-2.0

//! Encourages replacing `while(true)` with `loop` for infinite loops in Move
//! for clarity and conciseness. Identifies `while(true)` patterns, suggesting a
//! more idiomatic approach using `loop`. Aims to enhance code readability and
//! adherence to Rust idioms.

use super::{LinterDiagnosticCategory, LINT_WARNING_PREFIX, WHILE_TRUE_TO_LOOP_DIAG_CODE};
use crate::{
    diag,
    diagnostics::{
        codes::{custom, DiagnosticInfo, Severity},
        WarningFilters,
    },
    expansion::ast::Value_,
    shared::CompilationEnv,
    typing::{
        ast::{self as T, UnannotatedExp_},
        visitor::{TypingVisitorConstructor, TypingVisitorContext},
    },
};

const WHILE_TRUE_TO_LOOP_DIAG: DiagnosticInfo = custom(
    LINT_WARNING_PREFIX,
    Severity::Warning,
    LinterDiagnosticCategory::Complexity as u8,
    WHILE_TRUE_TO_LOOP_DIAG_CODE,
    "unnecessary 'while (true)', replace with 'loop'",
);

pub struct WhileTrueToLoop;

pub struct Context<'a> {
    env: &'a mut CompilationEnv,
}

impl TypingVisitorConstructor for WhileTrueToLoop {
    type Context<'a> = Context<'a>;

    fn context<'a>(env: &'a mut CompilationEnv, _program: &T::Program) -> Self::Context<'a> {
        Context { env }
    }
}

impl TypingVisitorContext for Context<'_> {
    fn add_warning_filter_scope(&mut self, filter: WarningFilters) {
        self.env.add_warning_filter_scope(filter)
    }
    fn pop_warning_filter_scope(&mut self) {
        self.env.pop_warning_filter_scope()
    }

    fn visit_exp_custom(&mut self, exp: &mut T::Exp) -> bool {
        let UnannotatedExp_::While(_, cond, _) = &exp.exp.value else {
            return false;
        };
        let UnannotatedExp_::Value(sp!(_, Value_::Bool(true))) = &cond.exp.value else {
            return false;
        };

        let msg = "'while (true)' can be always replaced with 'loop'";
        let mut diag = diag!(WHILE_TRUE_TO_LOOP_DIAG, (exp.exp.loc, msg));
        diag.add_note(
            "A 'loop' is more useful in these cases. Unlike 'while', 'loop' can have a \
            'break' with a value, e.g. 'let x = loop { break 42 };'",
        );
        self.env.add_diag(diag);

        false
    }
}
