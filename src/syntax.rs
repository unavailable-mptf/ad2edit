//! Chip code split into coloured spans: Expression 2 and, for Starfall,
//! Lua. The spans cover the text end to end in order, so an editor can lay
//! them out one after another. E2's rules are Wiremod's own lexer's: a
//! variable starts with a capital, a function with a small letter and is
//! followed by `(`, `#[ ]#` is a block comment and `#ifdef` and friends are
//! not comments at all.

use std::ops::Range;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Token {
    Plain,
    Comment,
    Text,
    Directive,
    Number,
    Keyword,
    Type,
    Function,
    Variable,
    Constant,
}

const E2_KEYWORDS: &[&str] = &[
    "if", "elseif", "else", "while", "for", "foreach", "switch", "case", "default", "break", "continue", "return", "local", "function", "do", "try", "catch", "event", "let", "const",
];

const E2_TYPES: &[&str] = &[
    "number", "normal", "string", "vector", "vector2", "vector4", "angle", "entity", "array", "table", "wirelink", "quaternion", "matrix", "matrix2", "matrix4", "ranger", "bone", "complex", "gtable", "void",
    "effect", "damage", "tracedata", "xwl", "xv2", "xv4", "xm2", "xm4", "xrd", "xgt", "xbo", "xxx",
];

const E2_PREPROCESSOR: &[&str] = &["#ifdef", "#ifndef", "#else", "#endif", "#include", "#error", "#warning"];

const LUA_KEYWORDS: &[&str] = &[
    "and", "break", "do", "else", "elseif", "end", "false", "for", "function", "goto", "if", "in", "local", "nil", "not", "or", "repeat", "return", "then", "true", "until", "while", "continue",
];

struct Spans<'a> {
    text: &'a str,
    found: Vec<(Range<usize>, Token)>,
}

impl Spans<'_> {
    fn push(&mut self, range: Range<usize>, token: Token) {
        if range.is_empty() {
            return;
        }
        match self.found.last_mut() {
            Some((last, kind)) if *kind == token && last.end == range.start => last.end = range.end,
            _ => self.found.push((range, token)),
        }
    }

    fn rest(&self, from: usize) -> &str {
        &self.text[from..]
    }

    fn word_end(&self, from: usize) -> usize {
        self.rest(from).find(|c: char| !(c.is_alphanumeric() || c == '_')).map_or(self.text.len(), |at| from + at)
    }

    fn number_end(&self, from: usize) -> usize {
        let rest = self.rest(from);
        let hex = rest.starts_with("0x") || rest.starts_with("0X") || rest.starts_with("0b") || rest.starts_with("0B");
        let mut end = from + if hex { 2 } else { 0 };
        let mut last = ' ';
        for (at, c) in self.rest(end).char_indices() {
            let digit = if hex { c.is_ascii_hexdigit() } else { c.is_ascii_digit() || c == '.' || c == 'e' || c == 'E' || ((c == '-' || c == '+') && (last == 'e' || last == 'E')) };
            if !digit {
                return end + at;
            }
            last = c;
        }
        end = self.text.len();
        end
    }

    fn line_end(&self, from: usize) -> usize {
        self.rest(from).find('\n').map_or(self.text.len(), |at| from + at)
    }

    /// The end of a quoted string opened at `from`, its closing quote
    /// included; a backslash escapes the next character.
    fn string_end(&self, from: usize, quote: char, spans_lines: bool) -> usize {
        let mut escaped = false;
        for (at, c) in self.rest(from + 1).char_indices() {
            if escaped {
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if c == quote {
                return from + 1 + at + c.len_utf8();
            } else if c == '\n' && !spans_lines {
                return from + 1 + at;
            }
        }
        self.text.len()
    }

    fn next_is_call(&self, from: usize) -> bool {
        self.rest(from).trim_start_matches([' ', '\t']).starts_with('(')
    }
}

/// Expression 2.
pub fn e2(text: &str) -> Vec<(Range<usize>, Token)> {
    let mut spans = Spans { text, found: Vec::new() };
    let mut at = 0;
    let mut line_start = true;
    while at < text.len() {
        let rest = spans.rest(at);
        let c = rest.chars().next().unwrap_or(' ');
        if rest.starts_with("#[") {
            let end = rest.find("]#").map_or(text.len(), |close| at + close + 2);
            spans.push(at..end, Token::Comment);
            at = end;
        } else if c == '#' {
            let end = spans.line_end(at);
            let word_end = spans.word_end(at + 1);
            let token = if E2_PREPROCESSOR.contains(&&text[at..word_end]) { Token::Directive } else { Token::Comment };
            if token == Token::Directive {
                spans.push(at..word_end, Token::Directive);
                spans.push(word_end..end, Token::Plain);
            } else {
                spans.push(at..end, Token::Comment);
            }
            at = end;
        } else if c == '@' && line_start {
            let word_end = spans.word_end(at + 1);
            spans.push(at..word_end, Token::Directive);
            let free_text = matches!(&text[at..word_end], "@name" | "@model");
            if free_text {
                let end = spans.line_end(word_end);
                spans.push(word_end..end, Token::Text);
                at = end;
            } else {
                at = word_end;
            }
        } else if c == '"' {
            let end = spans.string_end(at, '"', true);
            spans.push(at..end, Token::Text);
            at = end;
        } else if c.is_ascii_digit() {
            let end = spans.number_end(at);
            spans.push(at..end, Token::Number);
            at = end;
        } else if c.is_alphabetic() || c == '_' {
            let end = spans.word_end(at);
            let word = &text[at..end];
            let token = if c == '_' && word.len() > 1 && word[1..].chars().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_') {
                Token::Constant
            } else if c.is_uppercase() {
                Token::Variable
            } else if E2_KEYWORDS.contains(&word) {
                Token::Keyword
            } else if spans.next_is_call(end) {
                Token::Function
            } else if E2_TYPES.contains(&word) {
                Token::Type
            } else {
                Token::Plain
            };
            spans.push(at..end, token);
            at = end;
        } else {
            let end = at + c.len_utf8();
            spans.push(at..end, Token::Plain);
            at = end;
        }
        if c == '\n' {
            line_start = true;
        } else if !c.is_whitespace() {
            line_start = false;
        }
    }
    spans.found
}

/// Lua, as Starfall chips are written.
pub fn lua(text: &str) -> Vec<(Range<usize>, Token)> {
    let mut spans = Spans { text, found: Vec::new() };
    let mut at = 0;
    while at < text.len() {
        let rest = spans.rest(at);
        let c = rest.chars().next().unwrap_or(' ');
        let long_bracket = |from: &str| -> Option<usize> {
            let inner = from.strip_prefix('[')?;
            let level = inner.chars().take_while(|c| *c == '=').count();
            inner[level..].starts_with('[').then_some(level)
        };
        if rest.starts_with("--") {
            let end = match long_bracket(&rest[2..]) {
                Some(level) => {
                    let close = format!("]{}]", "=".repeat(level));
                    rest.find(&close).map_or(text.len(), |found| at + found + close.len())
                }
                None => spans.line_end(at),
            };
            spans.push(at..end, Token::Comment);
            at = end;
        } else if let Some(level) = long_bracket(rest) {
            let close = format!("]{}]", "=".repeat(level));
            let end = rest.find(&close).map_or(text.len(), |found| at + found + close.len());
            spans.push(at..end, Token::Text);
            at = end;
        } else if c == '"' || c == '\'' {
            let end = spans.string_end(at, c, false);
            spans.push(at..end, Token::Text);
            at = end;
        } else if c.is_ascii_digit() {
            let end = spans.number_end(at);
            spans.push(at..end, Token::Number);
            at = end;
        } else if c.is_alphabetic() || c == '_' {
            let end = spans.word_end(at);
            let word = &text[at..end];
            let token = if LUA_KEYWORDS.contains(&word) {
                Token::Keyword
            } else if spans.next_is_call(end) {
                Token::Function
            } else if word.len() > 1 && word.chars().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_') {
                Token::Constant
            } else {
                Token::Plain
            };
            spans.push(at..end, token);
            at = end;
        } else {
            let end = at + c.len_utf8();
            spans.push(at..end, Token::Plain);
            at = end;
        }
    }
    spans.found
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds<'a>(text: &'a str, spans: &[(Range<usize>, Token)]) -> Vec<(&'a str, Token)> {
        spans.iter().filter(|(range, _)| !text[range.clone()].trim().is_empty()).map(|(range, token)| (&text[range.clone()], *token)).collect()
    }

    fn covers(text: &str, spans: &[(Range<usize>, Token)]) -> bool {
        let mut at = 0;
        for (range, _) in spans {
            if range.start != at || range.end <= range.start || !text.is_char_boundary(range.end) {
                return false;
            }
            at = range.end;
        }
        at == text.len()
    }

    #[test]
    fn an_e2_is_split_the_way_wiremod_reads_it() {
        let code = "@name Flight core\n@inputs [Pod Cam]:wirelink Throttle\n@persist Speed:number\nif (first()) { Speed = _PI * 2.5 } # set up\nE = entity():pos()\n";
        let spans = e2(code);
        assert!(covers(code, &spans));
        let found = kinds(code, &spans);
        let has = |word: &str, token: Token| found.contains(&(word, token));
        assert!(has("@name", Token::Directive) && has(" Flight core", Token::Text));
        assert!(has("@inputs", Token::Directive) && has("Pod", Token::Variable) && has("wirelink", Token::Type) && has("Throttle", Token::Variable));
        assert!(has("number", Token::Type));
        assert!(has("if", Token::Keyword) && has("first", Token::Function) && has("_PI", Token::Constant) && has("2.5", Token::Number));
        assert!(has("# set up", Token::Comment));
        assert!(has("entity", Token::Function), "a type's name followed by ( is the function of that name");
        assert!(has("pos", Token::Function));
    }

    #[test]
    fn block_comments_strings_and_the_preprocessor_are_not_line_comments() {
        let code = "#[ two\nlines ]# X = \"a # not a comment \\\" still\nthe string\" #ifdef entity:setPos()\nprint(1)\n#endif\n@trigger none";
        let spans = e2(code);
        assert!(covers(code, &spans));
        let found = kinds(code, &spans);
        assert_eq!(found[0], ("#[ two\nlines ]#", Token::Comment));
        assert!(found.contains(&("\"a # not a comment \\\" still\nthe string\"", Token::Text)));
        assert!(found.contains(&("#ifdef", Token::Directive)) && found.contains(&("#endif", Token::Directive)));
        assert!(found.contains(&("@trigger", Token::Directive)));
        assert!(!found.iter().any(|(word, token)| word.contains("print") && *token == Token::Comment));
    }

    #[test]
    fn an_at_sign_mid_line_is_not_a_directive_and_odd_text_never_panics() {
        for code in ["X = Y @ Z", "", "\"unclosed", "#[ unclosed", "0x", "é = \"ü\" # ñ", "@", "#", "A\r\n@name B\r\n"] {
            assert!(covers(code, &e2(code)) || code.is_empty(), "{code:?}");
            assert!(covers(code, &lua(code)) || code.is_empty(), "{code:?}");
        }
        assert!(!kinds("X = Y @ Z", &e2("X = Y @ Z")).iter().any(|(_, token)| *token == Token::Directive));
    }

    #[test]
    fn lua_has_its_own_comments_strings_and_words() {
        let code = "--[[ a\nblock ]] local x = [==[long]==] -- tail\nhook.add('think', \"id\", function() return MAX_SPEED * 0xFF end)";
        let spans = lua(code);
        assert!(covers(code, &spans));
        let found = kinds(code, &spans);
        assert_eq!(found[0], ("--[[ a\nblock ]]", Token::Comment));
        assert!(found.contains(&("local", Token::Keyword)) && found.contains(&("[==[long]==]", Token::Text)) && found.contains(&("-- tail", Token::Comment)));
        assert!(found.contains(&("add", Token::Function)) && found.contains(&("'think'", Token::Text)) && found.contains(&("MAX_SPEED", Token::Constant)) && found.contains(&("0xFF", Token::Number)));
        assert!(found.contains(&("function", Token::Keyword)), "a keyword before ( stays a keyword");
    }
}
