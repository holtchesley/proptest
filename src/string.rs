//-
// Copyright 2017 Jason Lingle
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// http://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or http://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.

//! Strategies for generating strings and byte strings from regular
//! expressions.

use std::borrow::Cow;
use std::fmt;
use std::u32;

use regex_syntax as rs;

use bool;
use char;
use collection;
use bits;
use num;
use strategy::*;
use test_runner::*;

quick_error! {
    /// Errors which may occur when preparing a regular expression for use with
    /// string generation.
    #[derive(Debug)]
    pub enum Error {
        /// The string passed as the regex was not syntactically valid.
        RegexSyntax(err: rs::Error) {
            from()
            cause(err)
            description(err.description())
            display("{}", err)
        }
        /// The regex was syntactically valid, but contains elements not
        /// supported by proptest.
        UnsupportedRegex(message: &'static str) {
            description(message)
        }
    }
}

opaque_strategy_wrapper! {
    /// Strategy which generates values (i.e., `String` or `Vec<u8>`) matching
    /// a regular expression.
    ///
    /// Created by various functions in this module.
    #[derive(Debug)]
    pub struct RegexGeneratorStrategy[<T>][where T : fmt::Debug]
        (BoxedStrategy<T>) -> RegexGeneratorValueTree<T>;
    /// `ValueTree` corresponding to `RegexGeneratorStrategy`.
    pub struct RegexGeneratorValueTree[<T>][where T : fmt::Debug]
        (Box<ValueTree<Value = T>>) -> T;
}

impl Strategy for str {
    type Value = RegexGeneratorValueTree<String>;

    fn new_value(&self, runner: &mut TestRunner)
                 -> Result<Self::Value, String> {
        string_regex(self).unwrap().new_value(runner)
    }
}

/// Creates a strategy which generates strings matching the given regular
/// expression.
///
/// If you don't need error handling and aren't limited by setup time, it is
/// also possible to directly use a `&str` as a strategy with the same effect.
pub fn string_regex(regex: &str)
                    -> Result<RegexGeneratorStrategy<String>, Error> {
    string_regex_parsed(&rs::Expr::parse(regex)?)
}

/// Like `string_regex()`, but allows providing a pre-parsed expression.
pub fn string_regex_parsed(expr: &rs::Expr)
                           -> Result<RegexGeneratorStrategy<String>, Error> {
    bytes_regex_parsed(expr).map(
        |v| v.prop_map(|bytes| String::from_utf8(bytes).expect(
            "non-utf8 string")).boxed()).map(RegexGeneratorStrategy)
}

/// Creates a strategy which generates byte strings matching the given regular
/// expression.
pub fn bytes_regex(regex: &str)
                   -> Result<RegexGeneratorStrategy<Vec<u8>>, Error> {
    bytes_regex_parsed(&rs::Expr::parse(regex)?)
}

/// Like `bytes_regex()`, but allows providing a pre-parsed expression.
pub fn bytes_regex_parsed(expr: &rs::Expr)
                          -> Result<RegexGeneratorStrategy<Vec<u8>>, Error> {
    use self::rs::Expr::*;

    match *expr {
        Empty => Ok(Just(vec![]).boxed()),
        Literal { ref chars, casei: false } =>
            Ok(Just(chars.iter().map(|&c| c).collect::<String>()
                         .into_bytes()).boxed()),
        Literal { ref chars, casei: true } => {
            let chars = chars.to_owned();
            Ok(bits::bitset::between(0, chars.len())
               .prop_map(move |cases|
                         cases.into_bit_vec().iter().zip(chars.iter())
                         .map(|(case, &ch)| flip_case_to_bytes(case, ch))
                         .fold(vec![], |mut accum, rhs| {
                             accum.extend(rhs);
                             accum
                         }))
               .boxed())
        },
        LiteralBytes { ref bytes, casei: false } =>
            Ok(Just(bytes.to_owned()).boxed()),
        LiteralBytes { ref bytes, casei: true } => {
            let bytes = bytes.to_owned();
            Ok(bits::bitset::between(0, bytes.len())
               .prop_map(move |cases|
                         cases.into_bit_vec().iter().zip(bytes.iter())
                         .map(|(case, &byte)| flip_ascii_case(case, byte))
                         .collect::<Vec<_>>()).boxed())
        },

        AnyChar => Ok(char::ANY.boxed().prop_map(|c| to_bytes(c)).boxed()),
        AnyCharNoNL => {
            static NONL_RANGES: &[(char,char)] = &[
                ('\x00', '\x09'),
                // Multiple instances of the latter range to partially make up
                // for the bias of having such a tiny range in the control
                // characters.
                ('\x0B', ::std::char::MAX),
                ('\x0B', ::std::char::MAX),
                ('\x0B', ::std::char::MAX),
                ('\x0B', ::std::char::MAX),
                ('\x0B', ::std::char::MAX),
            ];
            Ok(char::ranges(Cow::Borrowed(NONL_RANGES))
               .prop_map(|c| to_bytes(c)).boxed())
        },
        AnyByte => Ok(num::u8::ANY.prop_map(|b| vec![b]).boxed()),
        AnyByteNoNL => Ok((0xBu8..).boxed()
                          .prop_union((..0xAu8).boxed())
                          .prop_map(|b| vec![b]).boxed()),

        Class(ref class) => {
            let ranges = (**class).iter().map(
                |&rs::ClassRange { start, end }| (start, end)).collect();
            Ok(char::ranges(Cow::Owned(ranges))
               .prop_map(to_bytes).boxed())
        }

        ClassBytes(ref class) => {
            let subs = (**class).iter().map(
                |&rs::ByteRange { start, end }| if 255u8 == end {
                    (start..).boxed()
                } else {
                    (start..end).boxed()
                }).collect::<Vec<_>>();
            Ok(Union::new(subs)
               .prop_map(|b| vec![b]).boxed())
        },

        Group { ref e, .. } => bytes_regex_parsed(e).map(|v| v.0),

        Repeat { ref e, r, .. } => {
            let range = match r {
                rs::Repeater::ZeroOrOne => 0..2,
                rs::Repeater::ZeroOrMore => 0..33,
                rs::Repeater::OneOrMore => 1..33,
                rs::Repeater::Range { min, max } => {
                    let max = if let Some(max) = max {
                        if u32::MAX == max {
                            return Err(Error::UnsupportedRegex(
                                "Cannot have repetition max of u32::MAX"));
                        } else {
                            max as usize + 1
                        }
                    } else if min < u32::MAX as u32 / 2 {
                        min as usize * 2
                    } else {
                        u32::MAX as usize
                    };

                    (min as usize)..max
                },
            };
            Ok(collection::vec(bytes_regex_parsed(e)?, range)
               .prop_map(|parts| parts.into_iter().fold(
                   vec![], |mut accum, child| { accum.extend(child); accum }))
               .boxed())
        },

        Concat(ref subs) => {
            let subs = subs.iter().map(|e| bytes_regex_parsed(e))
                .collect::<Result<Vec<_>, _>>()?;
            Ok(subs.into_iter()
               .fold(None, |accum, rhs| match accum {
                   None => Some(rhs.boxed()),
                   Some(accum) => Some(
                       (accum, rhs).prop_map(|(mut lhs, rhs)| {
                           lhs.extend(rhs);
                           lhs
                       }).boxed()),
               }).unwrap_or_else(
                   || Just(vec![]).boxed()))
        },

        Alternate(ref subs) => {
            let subs = subs.iter().map(|e| bytes_regex_parsed(e))
                .collect::<Result<Vec<_>, _>>()?;
            Ok(Union::new(subs).boxed())
        },

        StartLine |
        EndLine |
        StartText |
        EndText => Err(Error::UnsupportedRegex(
            "line/text anchors not supported for string generation")),

        WordBoundary |
        NotWordBoundary |
        WordBoundaryAscii |
        NotWordBoundaryAscii => Err(Error::UnsupportedRegex(
            "word boundary tests not supported for string generation")),
    }.map(RegexGeneratorStrategy)
}

fn flip_case_to_bytes(flip: bool, ch: char) -> Vec<u8> {
    if flip && ch.is_uppercase() {
        ch.to_lowercase().collect::<String>().into_bytes()
    } else if flip && ch.is_lowercase() {
        ch.to_uppercase().collect::<String>().into_bytes()
    } else {
        to_bytes(ch)
    }
}

fn to_bytes(ch: char) -> Vec<u8> {
    [ch].iter().map(|&c|c).collect::<String>().into_bytes()
}

fn flip_ascii_case(flip: bool, ch: u8) -> u8 {
    if flip && ch >= b'a' && ch <= b'z' {
        ch - b'a' + b'A'
    } else if flip && ch >= b'A' && ch <= b'Z' {
        ch - b'A' + b'a'
    } else {
        ch
    }
}

#[cfg(test)]
mod test {
    use std::collections::HashSet;

    use regex::Regex;

    use super::*;

    fn do_test(pattern: &str, min_distinct: usize, max_distinct: usize,
               iterations: usize) {
        let rx = Regex::new(pattern).unwrap();
        let mut generated = HashSet::new();

        let strategy = string_regex(pattern).unwrap();
        let mut runner = TestRunner::new(Config::default());
        for _ in 0..iterations {
            let mut value = strategy.new_value(&mut runner).unwrap();

            loop {
                let s = value.current();
                let ok = if let Some(matsch) = rx.find(&s) {
                    0 == matsch.start() && s.len() == matsch.end()
                } else {
                    false
                };
                if !ok {
                    panic!("Generated string {:?} which does not match {:?}",
                           s, pattern);
                }

                generated.insert(s);

                if !value.simplify() { break; }
            }
        }

        assert!(generated.len() >= min_distinct,
                "Expected to generate at least {} strings, but only \
                 generated {}", min_distinct, generated.len());
        assert!(generated.len() <= max_distinct,
                "Expected to generate at most {} strings, but \
                 generated {}", max_distinct, generated.len());
    }

    #[test]
    fn test_literal() {
        do_test("foo", 1, 1, 8);
    }

    #[test]
    fn test_casei_literal() {
        do_test("(?i:fOo)", 8, 8, 64);
    }

    #[test]
    fn test_alternation() {
        do_test("foo|bar|baz", 3, 3, 16);
    }

    #[test]
    fn test_repitition() {
        do_test("a{0,8}", 9, 9, 64);
    }

    #[test]
    fn test_question() {
        do_test("a?", 2, 2, 16);
    }

    #[test]
    fn test_start() {
        do_test("a*", 33, 33, 256);
    }

    #[test]
    fn test_plus() {
        do_test("a+", 32, 32, 256);
    }

    #[test]
    fn test_n_to_range() {
        do_test("a{4,}", 4, 4, 64);
    }

    #[test]
    fn test_concatenation() {
        do_test("(foo|bar)(xyzzy|plugh)", 4, 4, 32);
    }

    #[test]
    fn test_ascii_class() {
        do_test("[[:digit:]]", 10, 10, 64);
    }

    #[test]
    fn test_unicode_class() {
        do_test("\\p{Greek}", 24, 256, 64);
    }

    #[test]
    fn test_dot() {
        do_test(".", 200, 65536, 256);
    }

    #[test]
    fn test_dot_s() {
        do_test("(?s).", 200, 65536, 256);
    }
}
