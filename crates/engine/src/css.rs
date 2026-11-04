//! Tokenize CSS references without interpreting text inside strings/data SVGs.
use cssparser::{Parser, ParserInput, Token};

pub struct Reference {
    pub start: usize,
    pub end: usize,
    pub value: String,
    pub import: bool,
}

pub fn references(css: &str) -> Vec<Reference> {
    fn walk(input: &mut Parser<'_, '_>, output: &mut Vec<Reference>, depth: usize) {
        if depth > 64 {
            return;
        }
        let mut import = false;
        loop {
            let start = input.position().byte_index();
            let Ok(token) = input.next_including_whitespace_and_comments().cloned() else {
                break;
            };
            let value = match &token {
                Token::WhiteSpace(_) | Token::Comment(_) => continue,
                Token::AtKeyword(ref name) if name.eq_ignore_ascii_case("import") => {
                    import = true;
                    continue;
                }
                Token::UnquotedUrl(value) => Some(value.to_string()),
                Token::QuotedString(value) if import => Some(value.to_string()),
                Token::Function(ref name) if name.eq_ignore_ascii_case("url") => input
                    .parse_nested_block::<_, _, ()>(|nested| {
                        Ok(nested.expect_string_cloned()?.to_string())
                    })
                    .ok(),
                Token::Function(_)
                | Token::ParenthesisBlock
                | Token::SquareBracketBlock
                | Token::CurlyBracketBlock => {
                    let _ = input.parse_nested_block::<_, _, ()>(|nested| {
                        walk(nested, output, depth + 1);
                        Ok(())
                    });
                    None
                }
                _ => None,
            };
            if let Some(value) = value {
                output.push(Reference {
                    start,
                    end: input.position().byte_index(),
                    value,
                    import: import && matches!(token, Token::QuotedString(_)),
                });
            }
            import = false;
        }
    }
    let mut input = ParserInput::new(css);
    let mut output = Vec::new();
    walk(&mut Parser::new(&mut input), &mut output, 0);
    output
}

pub fn local(value: &str) -> bool {
    value.starts_with('#')
        || value
            .get(..5)
            .is_some_and(|s| s.eq_ignore_ascii_case("data:"))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn preserves_nested_svg_urls_comments_and_ordinary_strings() {
        let css = r##"a{mask:url("data:image/svg+xml,%3Csvg%3E%3Cg mask='url(%23mask)'/%3E");filter:url(#local);background:url("real (one).png");content:'url(fake.png)'}/* url(comment.png) */ @import 'theme.css';"##;
        let refs = references(css);
        assert_eq!(
            refs.iter().map(|r| r.value.as_str()).collect::<Vec<_>>(),
            [
                "data:image/svg+xml,%3Csvg%3E%3Cg mask='url(%23mask)'/%3E",
                "#local",
                "real (one).png",
                "theme.css"
            ]
        );
    }
}
