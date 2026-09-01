use lsp_types::CompletionItem;
use lsp_types::CompletionItemKind;
use lsp_types::CompletionList;
use lsp_types::InsertTextFormat;
use whim_syn::cst::node::NodeKind;
use whim_syn::token::kind::TokenKind;

use crate::server::analysis::Analysis;

struct Snippet {
    label: &'static str,
    detail: &'static str,
    text: &'static str,
}

const SNIPPETS: &[Snippet] = &[
    Snippet {
        label: "for",
        detail: "counted loop",
        text: "for (\\$${1:index} = 0; \\$${1:index} < ${2:count}; \\$${1:index}++) {\n\t$0\n}",
    },
    Snippet {
        label: "foreach",
        detail: "iterate values",
        text: "foreach (\\$${1:items} as \\$${2:item}) {\n\t$0\n}",
    },
    Snippet {
        label: "foreachkv",
        detail: "iterate keys and values",
        text: "foreach (\\$${1:items} as \\$${2:key} => \\$${3:value}) {\n\t$0\n}",
    },
    Snippet {
        label: "while",
        detail: "while loop",
        text: "while (${1:condition}) {\n\t$0\n}",
    },
    Snippet {
        label: "do",
        detail: "do-while loop",
        text: "do {\n\t$0\n} while (${1:condition});",
    },
    Snippet {
        label: "if",
        detail: "if statement",
        text: "if (${1:condition}) {\n\t$0\n}",
    },
    Snippet {
        label: "ifelse",
        detail: "if and else",
        text: "if (${1:condition}) {\n\t${2}\n} else {\n\t$0\n}",
    },
    Snippet {
        label: "try",
        detail: "try and catch",
        text: "try {\n\t${1}\n} catch (${2:Whim\\Unwind\\Throwable} \\$${3:error}) {\n\t$0\n}",
    },
    Snippet {
        label: "tryf",
        detail: "try, catch, and finally",
        text: "try {\n\t${1}\n} catch (${2:Whim\\Unwind\\Throwable} \\$${3:error}) {\n\t${4}\n} finally {\n\t$0\n}",
    },
    Snippet {
        label: "function",
        detail: "function declaration",
        text: "function ${1:name}(${2}): ${3:void} {\n\t$0\n}",
    },
    Snippet {
        label: "closure",
        detail: "closure with captures",
        text: "function (${1}) use (\\$${2:captured}): ${3:void} {\n\t$0\n}",
    },
    Snippet {
        label: "fn",
        detail: "short closure",
        text: "fn(${1}): ${2:mixed} => $0",
    },
    Snippet {
        label: "class",
        detail: "final class",
        text: "final class ${1:Name} {\n\t$0\n}",
    },
    Snippet {
        label: "classi",
        detail: "class with an interface",
        text: "final class ${1:Name} implements ${2:Interface} {\n\t$0\n}",
    },
    Snippet {
        label: "interface",
        detail: "interface declaration",
        text: "interface ${1:Name} {\n\t$0\n}",
    },
    Snippet {
        label: "enum",
        detail: "backed enum",
        text: "enum ${1:Name}: ${2:int} {\n\tcase ${3:First} = ${4:1};\n\t$0\n}",
    },
    Snippet {
        label: "type",
        detail: "type alias",
        text: "type ${1:Name} = ${2:mixed};",
    },
    Snippet {
        label: "newtype",
        detail: "newtype declaration",
        text: "newtype ${1:Name} = ${2:string};",
    },
    Snippet {
        label: "using",
        detail: "scoped resource",
        text: "using (\\$${1:resource} = ${2:new Resource()}) {\n\t$0\n}",
    },
    Snippet {
        label: "match",
        detail: "match expression",
        text: "match (\\$${1:subject}) {\n\t${2:1} => ${3:'first'},\n\t$0\n}",
    },
    Snippet {
        label: "method",
        detail: "public method",
        text: "public function ${1:name}(${2}): ${3:void} {\n\t$0\n}",
    },
    Snippet {
        label: "construct",
        detail: "constructor",
        text: "public function __construct(${1}) {\n\t$0\n}",
    },
    Snippet {
        label: "destruct",
        detail: "destructor",
        text: "public function __destruct(): void {\n\t$0\n}",
    },
    Snippet {
        label: "prop",
        detail: "property",
        text: "public ${1:string} \\$${2:name};",
    },
    Snippet {
        label: "propr",
        detail: "readonly property",
        text: "private readonly ${1:string} \\$${2:name};",
    },
    Snippet {
        label: "const",
        detail: "typed constant",
        text: "const ${1:int} ${2:NAME} = ${3:1};",
    },
    Snippet {
        label: "namespace",
        detail: "namespace declaration",
        text: "namespace ${1:Name};",
    },
    Snippet {
        label: "use",
        detail: "import",
        text: "use ${1:Namespace\\Name};",
    },
    Snippet {
        label: "return",
        detail: "return statement",
        text: "return $0;",
    },
    Snippet {
        label: "throw",
        detail: "throw expression",
        text: "throw new ${1:Whim\\Unwind\\RuntimeException}(${2:'message'});",
    },
];

pub(super) fn items(analysis: &Analysis<'_>, offset: usize) -> CompletionList {
    if inside_string_or_comment(analysis, offset) {
        return CompletionList::default();
    }

    let mut items = Vec::with_capacity(SNIPPETS.len() + TokenKind::ALL.len());
    if !inside_use(analysis, offset) {
        items.extend(SNIPPETS.iter().map(|snippet| CompletionItem {
            label: snippet.label.to_owned(),
            kind: Some(CompletionItemKind::SNIPPET),
            detail: Some(snippet.detail.to_owned()),
            insert_text: Some(snippet.text.to_owned()),
            insert_text_format: Some(InsertTextFormat::SNIPPET),
            sort_text: Some(format!("0{}", snippet.label)),
            ..CompletionItem::default()
        }));

        items.extend(TokenKind::ALL.iter().copied().filter_map(keyword));
    }

    CompletionList {
        is_incomplete: false,
        items,
    }
}

fn keyword(kind: TokenKind) -> Option<CompletionItem> {
    kind.reservation()?;
    let label = kind.describe().trim_matches('`');

    Some(CompletionItem {
        label: label.to_owned(),
        kind: Some(CompletionItemKind::KEYWORD),
        sort_text: Some(format!("1{label}")),
        ..CompletionItem::default()
    })
}

fn inside_use(analysis: &Analysis<'_>, offset: usize) -> bool {
    if analysis
        .elements()
        .iter()
        .any(|element| element.kind == NodeKind::Use && element.contains(offset))
    {
        return true;
    }

    for token in analysis.tokens().iter().rev() {
        if token.start.offset as usize >= offset || token.kind.is_comment() {
            continue;
        }

        match token.kind {
            TokenKind::Use => return true,
            TokenKind::Semicolon | TokenKind::LeftBrace | TokenKind::RightBrace => return false,
            _ => {}
        }
    }

    false
}

fn inside_string_or_comment(analysis: &Analysis<'_>, offset: usize) -> bool {
    analysis.tokens().iter().any(|token| {
        let start = token.start.offset as usize;
        let end = start + token.value.len();
        if token.kind.is_comment() || token.kind == TokenKind::Shebang {
            offset > start && offset <= end
        } else if matches!(token.kind, TokenKind::LiteralString | TokenKind::StringPart) {
            offset > start && offset < end
        } else {
            false
        }
    })
}

#[cfg(test)]
mod tests {
    use super::items;
    use crate::server::analysis::Analysis;

    fn labels(source: &str, offset: usize) -> Vec<String> {
        items(&Analysis::new(source), offset)
            .items
            .into_iter()
            .map(|item| item.label)
            .collect()
    }

    #[test]
    fn completion_uses_current_language_keywords() {
        let labels = labels("", 0);
        assert!(labels.contains(&"array".to_owned()));
        assert!(labels.contains(&"self".to_owned()));
        assert!(!labels.contains(&"defer".to_owned()));
        assert!(!labels.contains(&"instanceof".to_owned()));
    }

    #[test]
    fn strings_and_comments_have_no_completion() {
        let string = "'inside'";
        assert!(labels(string, 3).is_empty());
        let comment = "// inside";
        assert!(labels(comment, 5).is_empty());
    }

    #[test]
    fn removed_language_features_have_no_snippets() {
        assert!(!labels("", 0).contains(&"defer".to_owned()));
    }

    #[test]
    fn imports_do_not_offer_statement_snippets() {
        let complete = "use Whim\\Str;";
        assert!(labels(complete, complete.len() - 1).is_empty());

        let incomplete = "use Whim\\";
        assert!(labels(incomplete, incomplete.len()).is_empty());
    }
}
