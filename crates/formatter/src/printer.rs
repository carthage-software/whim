//! The document printer.

use std::str;

use whim_syn::arena::Arena;
use whim_syn::arena::Vec;

use crate::document::BreakMode;
use crate::document::Document;
use crate::document::Group;
use crate::document::IfBreak;
use crate::document::Line;
use crate::settings::FormatSettings;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    Break,
    Flat,
}

impl Mode {
    #[inline]
    const fn is_flat(self) -> bool {
        matches!(self, Self::Flat)
    }

    #[inline]
    const fn is_break(self) -> bool {
        matches!(self, Self::Break)
    }
}

/// The indentation prefix applied after a line break.
#[derive(Debug)]
enum Indentation<'arena, A>
where
    A: Arena,
{
    Root,
    Indent,
    Combined(Vec<'arena, Self, A>),
}

impl<A> Clone for Indentation<'_, A>
where
    A: Arena,
{
    fn clone(&self) -> Self {
        match self {
            Self::Root => Self::Root,
            Self::Indent => Self::Indent,
            Self::Combined(nested) => Self::Combined(nested.clone()),
        }
    }
}

impl<'arena, A> Indentation<'arena, A>
where
    A: Arena,
{
    fn get_value_in(&self, arena: &'arena A, use_tabs: bool, tab_width: usize) -> &'arena [u8] {
        match self {
            Indentation::Root => &[],
            Indentation::Indent => {
                if use_tabs {
                    b"\t"
                } else {
                    let mut spaces = Vec::with_capacity_in(tab_width, arena);
                    spaces.resize(tab_width, b' ');
                    spaces.leak()
                }
            }
            Indentation::Combined(nested) => {
                let mut combined = Vec::new_in(arena);
                for indentation in nested {
                    combined
                        .extend_from_slice(indentation.get_value_in(arena, use_tabs, tab_width));
                }
                combined.leak()
            }
        }
    }

    fn get_width_in(&self, tab_width: usize) -> usize {
        match self {
            Indentation::Root => 0,
            Indentation::Indent => tab_width,
            Indentation::Combined(nested) => nested
                .iter()
                .map(|indentation| indentation.get_width_in(tab_width))
                .sum(),
        }
    }
}

#[derive(Debug)]
enum Command<'arena, A>
where
    A: Arena,
{
    Print {
        indentation: Indentation<'arena, A>,
        mode: Mode,
        document: Document<'arena, A>,
    },
    BlankLineIfMultiline {
        indentation: Indentation<'arena, A>,
        starting_line: usize,
    },
}

impl<'arena, A> Command<'arena, A>
where
    A: Arena,
{
    #[inline]
    const fn new(
        indentation: Indentation<'arena, A>,
        mode: Mode,
        document: Document<'arena, A>,
    ) -> Self {
        Self::Print {
            indentation,
            mode,
            document,
        }
    }

    #[inline]
    const fn blank_line_if_multiline(
        indentation: Indentation<'arena, A>,
        starting_line: usize,
    ) -> Self {
        Self::BlankLineIfMultiline {
            indentation,
            starting_line,
        }
    }

    #[inline]
    const fn print_parts(&self) -> Option<(Mode, &Document<'arena, A>)> {
        match self {
            Self::Print { mode, document, .. } => Some((*mode, document)),
            Self::BlankLineIfMultiline { .. } => None,
        }
    }
}

pub(super) struct Printer<'arena, A>
where
    A: Arena,
{
    arena: &'arena A,
    settings: FormatSettings,
    out: Vec<'arena, u8, A>,
    position: usize,
    commands: Vec<'arena, Command<'arena, A>, A>,
    line_suffix: Vec<'arena, Command<'arena, A>, A>,
    new_line: &'static str,
    line: usize,
}

impl<'arena, A> Printer<'arena, A>
where
    A: Arena,
{
    #[must_use]
    pub(super) fn new(
        arena: &'arena A,
        document: Document<'arena, A>,
        capacity_hint: usize,
        settings: FormatSettings,
    ) -> Self {
        let out = Vec::with_capacity_in(capacity_hint, arena);
        let mut commands = Vec::new_in(arena);
        commands.push(Command::new(Indentation::Root, Mode::Break, document));

        Self {
            arena,
            settings,
            out,
            position: 0,
            commands,
            line_suffix: Vec::new_in(arena),
            new_line: settings.end_of_line.as_str(),
            line: 0,
        }
    }

    /// Prints the document, returning the arena-allocated output text.
    #[must_use]
    pub(super) fn build(mut self) -> &'arena str {
        let mut should_remeasure = false;

        while let Some(command) = self.commands.pop() {
            let (indentation, mode, document) = match command {
                Command::Print {
                    indentation,
                    mode,
                    document,
                } => (indentation, mode, document),
                Command::BlankLineIfMultiline {
                    indentation,
                    starting_line,
                } => {
                    if self.line > starting_line {
                        self.commands.push(Command::new(
                            indentation,
                            Mode::Break,
                            Document::Line(Line::hard()),
                        ));
                    }
                    continue;
                }
            };

            Self::propagate_breaks(&document);

            match document {
                Document::String(string) => self.handle_string(string),
                Document::Array(documents) => self.handle_array(&indentation, mode, documents),
                Document::Indent(documents) => self.handle_indent(&indentation, mode, documents),
                Document::IndentIfBreak(documents) => {
                    if mode.is_break() {
                        self.handle_indent(&indentation, mode, documents);
                    } else {
                        self.handle_array(&indentation, mode, documents);
                    }
                }
                Document::BlankLineAfterIfMultiline(document) => {
                    self.commands.push(Command::blank_line_if_multiline(
                        indentation.clone(),
                        self.line,
                    ));
                    self.commands.push(Command::new(
                        indentation,
                        mode,
                        clone_in_arena(self.arena, document),
                    ));
                }
                Document::Group(group) => {
                    should_remeasure =
                        self.handle_group(&indentation, mode, group, should_remeasure);
                }
                Document::Line(line) => {
                    should_remeasure =
                        self.handle_line(line, &indentation, mode, document, should_remeasure);
                }
                Document::LineSuffix(documents) => {
                    self.handle_line_suffix(indentation, mode, documents);
                }
                Document::IfBreak(if_break) => self.handle_if_break(if_break, indentation, mode),
                Document::BreakParent => {}
            }

            if self.commands.is_empty() && !self.line_suffix.is_empty() {
                self.commands.extend(self.line_suffix.drain(..).rev());
            }
        }

        // SAFETY: the printer only ever writes valid UTF-8 to its output buffer, so
        // it is safe to convert the buffer to a string without checking.
        unsafe { str::from_utf8_unchecked(self.out.leak()) }
    }

    #[inline]
    const fn remaining_width(&self) -> isize {
        (self.settings.print_width as isize) - (self.position as isize)
    }

    fn handle_string(&mut self, string: &'arena str) {
        self.out.extend_from_slice(string.as_bytes());
        self.position += string_width(string);
        self.line += string.bytes().filter(|byte| *byte == b'\n').count();
    }

    fn handle_array(
        &mut self,
        indentation: &Indentation<'arena, A>,
        mode: Mode,
        documents: Vec<'arena, Document<'arena, A>, A>,
    ) {
        self.commands.extend(
            documents
                .into_iter()
                .rev()
                .map(|document| Command::new(indentation.clone(), mode, document)),
        );
    }

    fn handle_indent(
        &mut self,
        indentation: &Indentation<'arena, A>,
        mode: Mode,
        documents: Vec<'arena, Document<'arena, A>, A>,
    ) {
        let mut nested = Vec::new_in(self.arena);
        nested.push(Indentation::Indent);
        nested.push(indentation.clone());
        let new_indentation = Indentation::Combined(nested);
        self.commands.extend(
            documents
                .into_iter()
                .rev()
                .map(|document| Command::new(new_indentation.clone(), mode, document)),
        );
    }

    fn handle_group(
        &mut self,
        indentation: &Indentation<'arena, A>,
        mode: Mode,
        group: Group<'arena, A>,
        mut should_remeasure: bool,
    ) -> bool {
        let should_break = match *group.break_mode.borrow() {
            BreakMode::Force => true,
            BreakMode::Parent => mode.is_break(),
            BreakMode::Auto | BreakMode::Never | BreakMode::Independent => false,
        };
        let never_break = matches!(*group.break_mode.borrow(), BreakMode::Never);

        if never_break {
            self.commands.extend(
                group
                    .contents
                    .into_iter()
                    .rev()
                    .map(|document| Command::new(indentation.clone(), Mode::Flat, document)),
            );
            return should_remeasure;
        }

        if mode.is_flat()
            && !should_remeasure
            && !matches!(*group.break_mode.borrow(), BreakMode::Independent)
        {
            let group_mode = if should_break { Mode::Break } else { mode };
            self.commands.extend(
                group
                    .contents
                    .into_iter()
                    .rev()
                    .map(|document| Command::new(indentation.clone(), group_mode, document)),
            );

            return should_remeasure;
        }

        should_remeasure = false;
        let remaining_width = self.remaining_width();
        let group_mode = if !should_break && self.group_fits(&group, remaining_width) {
            Mode::Flat
        } else {
            Mode::Break
        };
        self.commands.push(Command::new(
            indentation.clone(),
            group_mode,
            Document::Array(group.contents),
        ));

        should_remeasure
    }

    fn handle_line(
        &mut self,
        line: Line,
        indentation: &Indentation<'arena, A>,
        mode: Mode,
        document: Document<'arena, A>,
        mut should_remeasure: bool,
    ) -> bool {
        if mode.is_flat() {
            if !line.hard {
                if !line.soft {
                    self.out.push(b' ');
                    self.position += 1;
                }

                return should_remeasure;
            }

            should_remeasure = true;
        }

        if !self.line_suffix.is_empty() {
            self.commands
                .push(Command::new(indentation.clone(), mode, document));
            self.commands.extend(self.line_suffix.drain(..).rev());

            return should_remeasure;
        }

        self.trim_trailing_whitespace();
        self.out.extend_from_slice(self.new_line.as_bytes());
        self.position = self.add_indentation(indentation);
        self.line += 1;

        should_remeasure
    }

    fn handle_line_suffix(
        &mut self,
        indentation: Indentation<'arena, A>,
        mode: Mode,
        documents: Vec<'arena, Document<'arena, A>, A>,
    ) {
        self.line_suffix
            .push(Command::new(indentation, mode, Document::Array(documents)));
    }

    fn handle_if_break(
        &mut self,
        if_break: IfBreak<'arena, A>,
        indentation: Indentation<'arena, A>,
        mode: Mode,
    ) {
        let IfBreak {
            break_contents,
            flat_content,
        } = if_break;
        let contents = if mode.is_break() {
            break_contents
        } else {
            flat_content
        };
        self.commands.push(Command::new(
            indentation,
            mode,
            clone_in_arena(self.arena, contents),
        ));
    }

    fn add_indentation(&mut self, indentation: &Indentation<'arena, A>) -> usize {
        let value =
            indentation.get_value_in(self.arena, self.settings.use_tabs, self.settings.tab_width);
        self.out.extend_from_slice(value);

        indentation.get_width_in(self.settings.tab_width)
    }

    fn trim_trailing_whitespace(&mut self) {
        while let Some(&last) = self.out.last() {
            if last == b' ' || last == b'\t' {
                self.out.pop();
            } else {
                break;
            }
        }
    }

    /// Returns whether a flat group and the queued commands fit in `width`.
    fn group_fits(&self, group: &Group<'arena, A>, width: isize) -> bool {
        let mut remaining_width = width;
        let mut has_line_suffix = false;
        let mut stack: Vec<'arena, (Mode, &Document<'arena, A>), A> =
            Vec::with_capacity_in(16, self.arena);
        let mut commands = self.commands.iter().rev();

        for document in group.contents.iter().rev() {
            stack.push((Mode::Flat, document));
        }
        if stack.is_empty() {
            for command in commands.by_ref() {
                if let Some((mode, document)) = command.print_parts() {
                    stack.push((mode, document));
                    break;
                }
            }
        }

        while let Some((mode, document)) = stack.pop() {
            match document {
                Document::String(string) => remaining_width -= string_width(string) as isize,
                Document::Array(contents)
                | Document::Indent(contents)
                | Document::IndentIfBreak(contents) => {
                    for document in contents.iter().rev() {
                        stack.push((mode, document));
                    }
                }
                Document::Group(group) => {
                    let group_mode = match *group.break_mode.borrow() {
                        BreakMode::Force => Mode::Break,
                        BreakMode::Never => Mode::Flat,
                        BreakMode::Auto | BreakMode::Parent | BreakMode::Independent => mode,
                    };
                    for document in group.contents.iter().rev() {
                        stack.push((group_mode, document));
                    }
                }
                Document::IfBreak(if_break) => {
                    let contents = if mode.is_break() {
                        if_break.break_contents
                    } else {
                        if_break.flat_content
                    };
                    stack.push((mode, contents));
                }
                Document::Line(line) => {
                    if mode.is_break() || line.hard {
                        return true;
                    }

                    if !line.soft {
                        remaining_width -= 1;
                    }
                }
                Document::LineSuffix(_) => has_line_suffix = true,
                Document::BlankLineAfterIfMultiline(document) => {
                    stack.push((mode, document));
                }
                Document::BreakParent => {}
            }

            if remaining_width < 0 {
                return false;
            }

            if stack.is_empty() && !has_line_suffix {
                for command in commands.by_ref() {
                    if let Some((mode, document)) = command.print_parts() {
                        stack.push((mode, document));
                        break;
                    }
                }
            }
        }

        true
    }

    fn propagate_breaks(document: &Document<'_, A>) -> bool {
        let check_array = |documents: &Vec<'_, Document<'_, A>, A>| {
            documents.iter().rev().any(Self::propagate_breaks)
        };

        match document {
            Document::BreakParent => true,
            Document::Group(group) => {
                if matches!(*group.break_mode.borrow(), BreakMode::Never) {
                    return false;
                }

                let mut should_break = matches!(*group.break_mode.borrow(), BreakMode::Force);
                should_break |= check_array(&group.contents);

                if should_break {
                    group.break_mode.replace(BreakMode::Force);
                }

                matches!(*group.break_mode.borrow(), BreakMode::Force)
            }
            Document::IfBreak(if_break) => Self::propagate_breaks(if_break.break_contents),
            Document::Array(documents)
            | Document::Indent(documents)
            | Document::IndentIfBreak(documents) => check_array(documents),
            Document::BlankLineAfterIfMultiline(document) => Self::propagate_breaks(document),
            _ => false,
        }
    }
}

fn clone_in_arena<'arena, A>(
    arena: &'arena A,
    document: &Document<'arena, A>,
) -> Document<'arena, A>
where
    A: Arena,
{
    match document {
        Document::String(string) => Document::String(string),
        Document::Line(line) => Document::Line(*line),
        Document::BreakParent => Document::BreakParent,
        Document::Array(documents) => Document::Array(clone_vec_in_arena(arena, documents)),
        Document::Indent(documents) => Document::Indent(clone_vec_in_arena(arena, documents)),
        Document::IndentIfBreak(documents) => {
            Document::IndentIfBreak(clone_vec_in_arena(arena, documents))
        }
        Document::BlankLineAfterIfMultiline(document) => {
            Document::BlankLineAfterIfMultiline(arena.alloc(clone_in_arena(arena, document)))
        }
        Document::LineSuffix(documents) => {
            Document::LineSuffix(clone_vec_in_arena(arena, documents))
        }
        Document::Group(group) => Document::Group(Group {
            contents: clone_vec_in_arena(arena, &group.contents),
            break_mode: group.break_mode.clone(),
        }),
        Document::IfBreak(if_break) => Document::IfBreak(IfBreak {
            break_contents: arena.alloc(clone_in_arena(arena, if_break.break_contents)),
            flat_content: arena.alloc(clone_in_arena(arena, if_break.flat_content)),
        }),
    }
}

fn clone_vec_in_arena<'arena, A>(
    arena: &'arena A,
    source: &Vec<'arena, Document<'arena, A>, A>,
) -> Vec<'arena, Document<'arena, A>, A>
where
    A: Arena,
{
    let mut cloned = Vec::with_capacity_in(source.len(), arena);
    cloned.extend(
        source
            .iter()
            .map(|document| clone_in_arena(arena, document)),
    );
    cloned
}

/// The display width of `string`, measured on its last line.
#[must_use]
pub(super) fn string_width(string: &str) -> usize {
    use unicode_width::UnicodeWidthStr;

    let line = match string.rfind(['\n', '\r']) {
        Some(index) => &string[index + 1..],
        None => string,
    };

    UnicodeWidthStr::width(line)
}
