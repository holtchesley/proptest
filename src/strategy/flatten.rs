//-
// Copyright 2017 Jason Lingle
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// http://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or http://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.

use std::fmt;
use std::mem;
use std::sync::Arc;

use strategy::traits::*;
use test_runner::*;

/// Adaptor that flattens a `Strategy` which produces other `Strategy`s into a
/// `Strategy` that picks one of those strategies and then picks values from
/// it.
#[derive(Debug, Clone, Copy)]
pub struct Flatten<S> {
    source: S,
}

impl<S : Strategy> Flatten<S> {
    /// Wrap `source` to flatten it.
    pub fn new(source: S) -> Self {
        Flatten { source }
    }
}

impl<S : Strategy> Strategy for Flatten<S>
where <S::Value as ValueTree>::Value : Strategy {
    type Value = FlattenValueTree<S::Value>;

    fn new_value(&self, runner: &mut TestRunner)
                 -> Result<Self::Value, String> {
        let meta = self.source.new_value(runner)?;
        FlattenValueTree::new(runner, meta)
    }
}

/// The `ValueTree` produced by `Flatten`.
pub struct FlattenValueTree<S : ValueTree> where S::Value : Strategy {
    meta: S,
    current: <S::Value as Strategy>::Value,
    // The final value to produce after successive calls to complicate() on the
    // underlying objects return false.
    final_complication: Option<<S::Value as Strategy>::Value>,
    // When `simplify()` or `complicate()` causes a new `Strategy` to be
    // chosen, we need to find a new failing input for that case. To do this,
    // we implement `complicate()` by regenerating values up to a number of
    // times corresponding to the maximum number of test cases. A `simplify()`
    // which does not cause a new strategy to be chosen always resets
    // `complicate_regen_remaining` to 0.
    //
    // This does unfortunately depart from the direct interpretation of
    // simplify/complicate as binary search, but is still easier to think about
    // than other implementations of higher-order strategies.
    runner: TestRunner,
    complicate_regen_remaining: u32,
}

impl<S : ValueTree> fmt::Debug for FlattenValueTree<S>
where S::Value : Strategy,
      S : fmt::Debug, <S::Value as Strategy>::Value : fmt::Debug {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.debug_struct("FlattenValueTree")
            .field("meta", &self.meta)
            .field("current", &self.current)
            .field("final_complication", &self.final_complication)
            .field("complicate_regen_remaining",
                   &self.complicate_regen_remaining)
            .finish()
    }
}

impl<S : ValueTree> FlattenValueTree<S> where S::Value : Strategy {
    fn new(runner: &mut TestRunner, meta: S) -> Result<Self, String> {
        let current = meta.current().new_value(runner)?;
        Ok(FlattenValueTree {
            meta, current,
            final_complication: None,
            runner: runner.partial_clone(),
            complicate_regen_remaining: 0
        })
    }
}

impl<S : ValueTree> ValueTree for FlattenValueTree<S>
where S::Value : Strategy {
    type Value = <<S::Value as Strategy>::Value as ValueTree>::Value;

    fn current(&self) -> Self::Value {
        self.current.current()
    }

    fn simplify(&mut self) -> bool {
        self.complicate_regen_remaining = 0;

        if self.current.simplify() {
            true
        } else if !self.meta.simplify() {
            false
        } else {
            match self.meta.current().new_value(&mut self.runner) {
                Ok(v) => {
                    // Shift current into final_complication and `v` into
                    // `current`.
                    self.final_complication = Some(v);
                    mem::swap(self.final_complication.as_mut().unwrap(),
                              &mut self.current);
                    // Initially complicate by regenerating the chosen value.
                    self.complicate_regen_remaining =
                        self.runner.config().cases;
                    true
                },
                Err(_) => false,
            }
        }
    }

    fn complicate(&mut self) -> bool {
        if self.complicate_regen_remaining > 0 {
            if self.runner.flat_map_regen() {
                self.complicate_regen_remaining -= 1;

                if let Ok(v) = self.meta.current().new_value(&mut self.runner) {
                    self.current = v;
                    return true;
                }
            } else {
                self.complicate_regen_remaining = 0;
            }
        }

        let res = if self.current.complicate() {
            true
        } else if self.meta.complicate() {
            match self.meta.current().new_value(&mut self.runner) {
                Ok(v) => {
                    self.complicate_regen_remaining =
                        self.runner.config().cases;
                    self.current = v;
                    true
                },
                Err(_) => false,
            }
        } else {
            false
        };

        if res {
            true
        } else if let Some(v) = self.final_complication.take() {
            self.current = v;
            true
        } else {
            false
        }
    }
}

/// Similar to `Flatten`, but does not shrink the input strategy.
///
/// See `Strategy::prop_ind_flat_map()` fore more details.
#[derive(Clone, Copy, Debug)]
pub struct IndFlatten<S>(pub(super) S);

impl<S : Strategy> Strategy for IndFlatten<S>
where <S::Value as ValueTree>::Value : Strategy {
    type Value = <<S::Value as ValueTree>::Value as Strategy>::Value;

    fn new_value(&self, runner: &mut TestRunner)
                 -> Result<Self::Value, String> {
        let inner = self.0.new_value(runner)?;
        inner.current().new_value(runner)
    }
}

/// Similar to `Map` plus `Flatten`, but does not shrink the input strategy and
/// passes the original input through.
///
/// See `Strategy::prop_ind_flat_map2()` for more details.
pub struct IndFlattenMap<S, F> {
    pub(super) source: S,
    pub(super) fun: Arc<F>,
}

impl<S : fmt::Debug, F> fmt::Debug for IndFlattenMap<S, F> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.debug_struct("IndFlattenMap")
            .field("source", &self.source)
            .field("fun", &"<function>")
            .finish()
    }
}

impl<S : Clone, F> Clone for IndFlattenMap<S, F> {
    fn clone(&self) -> Self {
        IndFlattenMap {
            source: self.source.clone(),
            fun: self.fun.clone(),
        }
    }
}

impl<S : Strategy, R : Strategy,
     F : Fn (<S::Value as ValueTree>::Value) -> R>
Strategy for IndFlattenMap<S, F> {
    type Value = ::tuple::TupleValueTree<(S::Value, R::Value)>;

    fn new_value(&self, runner: &mut TestRunner)
                 -> Result<Self::Value, String> {
        let left = self.source.new_value(runner)?;
        let right_source = (self.fun)(left.current());
        let right = right_source.new_value(runner)?;

        Ok(::tuple::TupleValueTree::new((left, right)))
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_flat_map() {
        // Pick random integer A, then random integer B which is ±5 of A and
        // assert that B <= A if A > 10000. Shrinking should always converge to
        // A=10001, B=10002.
        let input = (0..65536).prop_flat_map(
            |a| (Just(a), (a-5..a+5)));

        let mut failures = 0;
        for _ in 0..1000 {
            let mut runner = TestRunner::new(Config::default());
            let case = input.new_value(&mut runner).unwrap();
            let result = runner.run_one(case, |&(a, b)| {
                if a <= 10000 || b <= a {
                    Ok(())
                } else {
                    Err(TestCaseError::Fail("fail".to_owned()))
                }
            });

            match result {
                Ok(_) => { },
                Err(TestError::Fail(_, v)) => {
                    failures += 1;
                    assert_eq!((10001, 10002), v);
                },
                result => panic!("Unexpected result: {:?}", result),
            }
        }

        assert!(failures > 250);
    }

    #[test]
    fn flat_map_respects_regen_limit() {
        use std::sync::atomic::{AtomicBool, Ordering};

        let input = (0..65536)
            .prop_flat_map(|_| 0..65536)
            .prop_flat_map(|_| 0..65536)
            .prop_flat_map(|_| 0..65536)
            .prop_flat_map(|_| 0..65536)
            .prop_flat_map(|_| 0..65536);

        // Arteficially make the first case fail and all others pass, so that
        // the regeneration logic futilely searches for another failing
        // example and eventually gives up. Unfortunately, the test is sort of
        // semi-decidable; if the limit *doesn't* work, the test just runs
        // almost forever.
        let pass = AtomicBool::new(false);
        let mut runner = TestRunner::new(Config {
            max_flat_map_regens: 1000,
            .. Config::default()
        });
        let case = input.new_value(&mut runner).unwrap();
        let _ = runner.run_one(case, |_| {
            if pass.fetch_or(true, Ordering::SeqCst) {
                Ok(())
            } else {
                Err(TestCaseError::Fail("fail".to_owned()))
            }
        });
    }
}
