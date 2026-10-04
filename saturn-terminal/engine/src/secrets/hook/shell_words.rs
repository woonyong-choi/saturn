//! 셸 명령을 따옴표와 이스케이프를 푼 낱말로 나눈다. 판정은 `hook`이 한다.

/// 연결 기호로 나뉜 단순 명령 하나.
#[derive(Debug, Default)]
pub(super) struct Segment {
    /// 따옴표와 이스케이프를 푼 낱말.
    pub(super) words: Vec<String>,
    /// 앞 명령의 출력을 파이프로 받는다.
    pub(super) piped: bool,
    /// `<<`로 이어진 본문.
    pub(super) heredoc: Option<String>,
}

/// 따옴표 짝이나 `<<` 구분자가 없어 해석할 수 없으면 `None`이다.
pub(super) fn tokenize(input: &str) -> Option<Vec<Segment>> {
    let mut lexer = Lexer {
        chars: input.chars().collect(),
        pos: 0,
        segments: Vec::new(),
        current: Segment::default(),
        word: String::new(),
        in_word: false,
        pending: Vec::new(),
    };
    while let Some(c) = lexer.peek(0) {
        lexer.step(c)?;
    }
    lexer.end_segment(false);
    Some(lexer.segments)
}

struct PendingHeredoc {
    delimiter: String,
    strip_tabs: bool,
    /// 본문을 붙일 단순 명령의 번호.
    segment: usize,
}

struct Lexer {
    chars: Vec<char>,
    pos: usize,
    segments: Vec<Segment>,
    current: Segment,
    word: String,
    in_word: bool,
    pending: Vec<PendingHeredoc>,
}

impl Lexer {
    fn peek(&self, ahead: usize) -> Option<char> {
        self.chars.get(self.pos + ahead).copied()
    }

    fn step(&mut self, c: char) -> Option<()> {
        match c {
            '\\' => self.backslash(),
            '\'' => self.single_quoted()?,
            '"' => self.double_quoted()?,
            '#' if !self.in_word => self.skip_comment(),
            '\n' => self.newline(),
            ';' | '`' | '(' | ')' => self.separator(1, false),
            '&' => self.separator(if self.peek(1) == Some('&') { 2 } else { 1 }, false),
            '|' => {
                let double = self.peek(1) == Some('|');
                self.separator(if double { 2 } else { 1 }, !double);
            }
            '$' if self.peek(1) == Some('(') => self.separator(2, false),
            '<' if self.starts_heredoc() => self.heredoc_operator()?,
            c if c.is_whitespace() => {
                self.end_word();
                self.pos += 1;
            }
            c => self.push(c),
        }
        Some(())
    }

    fn push(&mut self, c: char) {
        self.word.push(c);
        self.in_word = true;
        self.pos += 1;
    }

    fn end_word(&mut self) {
        if self.in_word {
            self.current.words.push(std::mem::take(&mut self.word));
            self.in_word = false;
        }
    }

    fn end_segment(&mut self, piped_next: bool) {
        self.end_word();
        if !self.current.words.is_empty() {
            self.segments.push(std::mem::take(&mut self.current));
        }
        self.current.piped = piped_next;
    }

    fn separator(&mut self, len: usize, piped_next: bool) {
        self.end_segment(piped_next);
        self.pos += len;
    }

    fn backslash(&mut self) {
        match self.peek(1) {
            Some('\n') => self.pos += 2,
            Some(escaped) => {
                self.word.push(escaped);
                self.in_word = true;
                self.pos += 2;
            }
            None => self.pos += 1,
        }
    }

    fn single_quoted(&mut self) -> Option<()> {
        let start = self.pos + 1;
        let len = self.chars[start..].iter().position(|&c| c == '\'')?;
        self.word.extend(&self.chars[start..start + len]);
        self.in_word = true;
        self.pos = start + len + 1;
        Some(())
    }

    fn double_quoted(&mut self) -> Option<()> {
        self.pos += 1;
        loop {
            let c = self.peek(0)?;
            self.pos += 1;
            match c {
                '"' => break,
                '\\' => self.double_quote_escape()?,
                _ => self.word.push(c),
            }
        }
        self.in_word = true;
        Some(())
    }

    fn double_quote_escape(&mut self) -> Option<()> {
        let escaped = self.peek(0)?;
        self.pos += 1;
        match escaped {
            '\n' => {}
            '"' | '\\' | '$' | '`' => self.word.push(escaped),
            other => {
                self.word.push('\\');
                self.word.push(other);
            }
        }
        Some(())
    }

    fn skip_comment(&mut self) {
        while self.peek(0).is_some_and(|c| c != '\n') {
            self.pos += 1;
        }
    }

    fn starts_heredoc(&self) -> bool {
        self.peek(1) == Some('<')
            && self.peek(2) != Some('<')
            && (self.pos == 0 || self.chars[self.pos - 1] != '<')
    }

    fn heredoc_operator(&mut self) -> Option<()> {
        self.end_word();
        self.pos += 2;
        let strip_tabs = self.peek(0) == Some('-');
        if strip_tabs {
            self.pos += 1;
        }
        while matches!(self.peek(0), Some(' ' | '\t')) {
            self.pos += 1;
        }
        let mut delimiter = String::new();
        while let Some(c) = self.peek(0) {
            if c.is_whitespace() || ";&|()<>".contains(c) {
                break;
            }
            if !matches!(c, '\'' | '"' | '\\') {
                delimiter.push(c);
            }
            self.pos += 1;
        }
        if delimiter.is_empty() {
            return None;
        }
        self.pending.push(PendingHeredoc {
            delimiter,
            strip_tabs,
            segment: self.segments.len(),
        });
        Some(())
    }

    fn newline(&mut self) {
        self.end_segment(false);
        self.pos += 1;
        for heredoc in std::mem::take(&mut self.pending) {
            let body = self.read_heredoc_body(&heredoc);
            if let Some(segment) = self.segments.get_mut(heredoc.segment) {
                segment
                    .heredoc
                    .get_or_insert_with(String::new)
                    .push_str(&body);
            }
        }
    }

    /// 구분자 줄이 없으면 끝까지를 본문으로 본다.
    fn read_heredoc_body(&mut self, heredoc: &PendingHeredoc) -> String {
        let mut body = String::new();
        while self.pos < self.chars.len() {
            let end = self.chars[self.pos..]
                .iter()
                .position(|&c| c == '\n')
                .map_or(self.chars.len(), |len| self.pos + len);
            let line: String = self.chars[self.pos..end].iter().collect();
            self.pos = (end + 1).min(self.chars.len());
            let candidate = if heredoc.strip_tabs {
                line.trim_start_matches('\t')
            } else {
                line.as_str()
            };
            if candidate == heredoc.delimiter {
                break;
            }
            body.push_str(&line);
            body.push('\n');
        }
        body
    }
}
