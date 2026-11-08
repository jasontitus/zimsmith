//! Tokenize CSS references without interpreting text inside strings/data SVGs.
use cssparser::{Delimiter, Parser, ParserInput, Token};

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

// Each src list describes interchangeable encodings of one face. Different
// weights, styles and unicode ranges live in separate rules and stay separate.
pub fn compact_font_sources(css: &str, saved: impl Fn(&str) -> bool) -> String {
    fn walk<'i>(
        input: &mut Parser<'i, '_>,
        edits: &mut Vec<(usize, usize, String)>,
        saved: &impl Fn(&str) -> bool,
        depth: usize,
    ) {
        if depth > 64 {
            return;
        }
        let mut font_face = false;
        while let Ok(token) = input.next().cloned() {
            match token {
                Token::AtKeyword(name) => font_face = name.eq_ignore_ascii_case("font-face"),
                Token::Semicolon => font_face = false,
                Token::CurlyBracketBlock => {
                    let _ = input.parse_nested_block::<_, _, ()>(|block| {
                        if font_face {
                            compact_face(block, edits, saved);
                        } else {
                            walk(block, edits, saved, depth + 1);
                        }
                        Ok(())
                    });
                    font_face = false;
                }
                _ => {}
            }
        }
    }
    fn compact_face<'i>(
        block: &mut Parser<'i, '_>,
        edits: &mut Vec<(usize, usize, String)>,
        saved: &impl Fn(&str) -> bool,
    ) {
        while !block.is_exhausted() {
            let _ = block.parse_until_after::<_, _, ()>(Delimiter::Semicolon, |declaration| {
                declaration.expect_ident_matching("src")?;
                declaration.expect_colon()?;
                let start = declaration.position().byte_index();
                let alternatives = declaration.parse_comma_separated::<_, _, ()>(|part| {
                    let start = part.position();
                    while part.next().is_ok() {}
                    Ok(part.slice_from(start))
                })?;
                let end = declaration.position().byte_index();
                let sources: Vec<_> = alternatives.iter().map(|part| references(part)).collect();
                if sources.iter().any(|refs| refs.len() > 1)
                    || alternatives
                        .iter()
                        .any(|part| part.to_ascii_lowercase().contains("tech("))
                {
                    return Ok(());
                }
                let remote: Vec<_> = sources
                    .iter()
                    .enumerate()
                    .filter_map(|(i, refs)| {
                        let url = &refs.first()?.value;
                        (!local(url)).then_some((i, url.as_str()))
                    })
                    .collect();
                if remote.len() < 2 {
                    return Ok(());
                }
                let chosen = remote
                    .iter()
                    .min_by_key(|(i, url)| {
                        let path = url
                            .split(['?', '#'])
                            .next()
                            .unwrap_or(url)
                            .to_ascii_lowercase();
                        let format = if path.ends_with(".woff2") {
                            0
                        } else if path.ends_with(".woff") {
                            1
                        } else if path.ends_with(".ttf") || path.ends_with(".otf") {
                            2
                        } else {
                            3
                        };
                        (!saved(url), format, *i)
                    })
                    .unwrap()
                    .0;
                let kept = alternatives
                    .iter()
                    .enumerate()
                    .filter(|(i, _)| *i == chosen || !remote.iter().any(|(index, _)| index == i))
                    .map(|(_, part)| part.trim())
                    .collect::<Vec<_>>()
                    .join(", ");
                edits.push((start, end, format!(" {kept}")));
                Ok(())
            });
        }
    }
    let mut input = ParserInput::new(css);
    let mut edits = Vec::new();
    walk(&mut Parser::new(&mut input), &mut edits, &saved, 0);
    if edits.is_empty() {
        return css.into();
    }
    let mut result = css.to_owned();
    for (start, end, value) in edits.into_iter().rev() {
        result.replace_range(start..end, &value);
    }
    result
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
