use whim_span::Span;
use whim_syn::cst::trivia::Trivia;

pub(crate) fn comment_lines<'a>(trivia: &'a Trivia<'_>) -> impl Iterator<Item = (Span, &'a str)> {
    let text = trivia.value.trim_start_matches('/');
    let start = trivia.span.start + (trivia.value.len() - text.len()) as u32;
    text.trim_end_matches("*/")
        .split_inclusive('\n')
        .scan(start, |offset, line| {
            let text = line.trim_start().trim_start_matches('*').trim_start();
            let start = *offset + (line.len() - text.len()) as u32;
            *offset += line.len() as u32;
            let text = text.trim_end_matches(['\r', '\n']);
            Some((Span::new(start, start + text.len() as u32), text))
        })
}
