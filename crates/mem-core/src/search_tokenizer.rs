use anyhow::Result;
use tantivy::Index;

#[cfg(feature = "cjk")]
use std::borrow::Cow;
#[cfg(feature = "cjk")]
use anyhow::Context;
#[cfg(feature = "cjk")]
use tantivy::tokenizer::{Token, TokenStream, Tokenizer};

#[cfg(feature = "cjk")]
use lindera::dictionary::load_dictionary;
#[cfg(feature = "cjk")]
use lindera::mode::Mode;
#[cfg(feature = "cjk")]
use lindera::segmenter::Segmenter;

pub(crate) fn register(index: &Index) -> Result<()> {
    #[cfg(feature = "cjk")]
    {
        let dictionary =
            load_dictionary("embedded://cc-cedict").context("load embedded CC-CEDICT")?;
        let segmenter = Segmenter::new(Mode::Normal, dictionary, None);
        index
            .tokenizers()
            .register("multilingual", LinderaTokenizer::new(segmenter));
    }
    #[cfg(not(feature = "cjk"))]
    {
        use tantivy::tokenizer::SimpleTokenizer;
        index
            .tokenizers()
            .register("multilingual", SimpleTokenizer::default());
    }
    Ok(())
}

#[cfg(feature = "cjk")]
#[derive(Clone)]
struct LinderaTokenizer {
    segmenter: Segmenter,
}

#[cfg(feature = "cjk")]
impl LinderaTokenizer {
    fn new(segmenter: Segmenter) -> Self {
        Self { segmenter }
    }
}

#[cfg(feature = "cjk")]
impl Tokenizer for LinderaTokenizer {
    type TokenStream<'a> = LinderaTokenStream;

    fn token_stream<'a>(&'a mut self, text: &'a str) -> Self::TokenStream<'a> {
        let tokens = self
            .segmenter
            .segment(Cow::Borrowed(text))
            .unwrap_or_default()
            .into_iter()
            .enumerate()
            .map(|(position, token)| Token {
                offset_from: token.byte_start,
                offset_to: token.byte_end,
                position,
                text: token.surface.to_string(),
                position_length: token.position_length,
            })
            .collect();
        LinderaTokenStream {
            tokens,
            current: Token::default(),
            next_index: 0,
        }
    }
}

#[cfg(feature = "cjk")]
struct LinderaTokenStream {
    tokens: Vec<Token>,
    current: Token,
    next_index: usize,
}

#[cfg(feature = "cjk")]
impl TokenStream for LinderaTokenStream {
    fn advance(&mut self) -> bool {
        let Some(token) = self.tokens.get(self.next_index) else {
            return false;
        };
        self.current = token.clone();
        self.next_index += 1;
        true
    }

    fn token(&self) -> &Token {
        &self.current
    }

    fn token_mut(&mut self) -> &mut Token {
        &mut self.current
    }
}
