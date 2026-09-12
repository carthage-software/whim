use whim_syn::cst::trivia::Trivia;

pub(crate) fn comment_lines<'a>(trivia: &'a Trivia<'_>) -> impl Iterator<Item = &'a str> {
    trivia
        .value
        .trim_start_matches('/')
        .trim_end_matches("*/")
        .lines()
        .map(|line| line.trim_start().trim_start_matches('*').trim_start())
}
