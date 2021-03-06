//-
// Copyright 2017 Jason Lingle
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// http://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or http://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.

//! Strategies for combining delegate strategies into `std::Result`s.
//!
//! That is, the strategies here are for producing `Ok` _and_ `Err` cases. To
//! simply adapt a strategy producing `T` into `Result<T, something>` which is
//! always `Ok`, you can do something like `base_strategy.prop_map(Ok)` to
//! simply wrap the generated values.
//!
//! Note that there are two nearly identical APIs for doing this, termed "maybe
//! ok" and "maybe err". The difference between the two is in how they shrink;
//! "maybe ok" treats `Ok` as the special case and shrinks to `Err`;
//! conversely, "maybe err" treats `Err` as the special case and shrinks to
//! `Ok`. Which to use largely depends on the code being tested; if the code
//! typically handles errors by immediately bailing out and doing nothing else,
//! "maybe ok" is likely more suitable, as shrinking will cause the code to
//! take simpler paths. On the other hand, functions that need to make a
//! complicated or fragile "back out" process on error are better tested with
//! "maybe err" since the success case results in an easier to understand code
//! path.

use std::fmt;
use std::marker::PhantomData;

use strategy::*;
use test_runner::*;

struct WrapOk<T, E>(PhantomData<T>, PhantomData<E>);
impl<T, E> Clone for WrapOk<T, E> {
    fn clone(&self) -> Self { *self }
}
impl<T, E> Copy for WrapOk<T, E> { }
impl<T, E> fmt::Debug for WrapOk<T, E> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "WrapOk")
    }
}
impl<T : fmt::Debug, E : fmt::Debug> statics::MapFn<T> for WrapOk<T, E> {
    type Output = Result<T, E>;
    fn apply(&self, t: T) -> Result<T, E> {
        Ok(t)
    }
}
struct WrapErr<T, E>(PhantomData<T>, PhantomData<E>);
impl<T, E> Clone for WrapErr<T, E> {
    fn clone(&self) -> Self { *self }
}
impl<T, E> Copy for WrapErr<T, E> { }
impl<T, E> fmt::Debug for WrapErr<T, E> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "WrapErr")
    }
}
impl<T : fmt::Debug, E : fmt::Debug> statics::MapFn<E> for WrapErr<T, E> {
    type Output = Result<T, E>;
    fn apply(&self, e: E) -> Result<T, E> {
        Err(e)
    }
}

opaque_strategy_wrapper! {
    /// Strategy which generates `Result`s using `Ok` and `Err` values from two
    /// delegate strategies.
    ///
    /// Shrinks to `Err`.
    #[derive(Clone)]
    pub struct MaybeOk[<T, E>][where T : Strategy, E : Strategy]
        (TupleUnion<((u32, statics::Map<E, WrapErr<<T::Value as ValueTree>::Value,
                                                   <E::Value as ValueTree>::Value>>),
                     (u32, statics::Map<T, WrapOk<<T::Value as ValueTree>::Value,
                                                  <E::Value as ValueTree>::Value>>))>)
        -> MaybeOkValueTree<T::Value, E::Value>;
    /// `ValueTree` type corresponding to `MaybeOk`.
    #[derive(Clone, Debug)]
    pub struct MaybeOkValueTree[<T, E>][where T : ValueTree, E : ValueTree]
        (TupleUnionValueTree<(statics::Map<E, WrapErr<T::Value, E::Value>>,
                              Option<statics::Map<T, WrapOk<T::Value, E::Value>>>)>)
        -> Result<T::Value, E::Value>;
}

opaque_strategy_wrapper! {
    /// Strategy which generates `Result`s using `Ok` and `Err` values from two
    /// delegate strategies.
    ///
    /// Shrinks to `Ok`.
    #[derive(Clone)]
    pub struct MaybeErr[<T, E>][where T : Strategy, E : Strategy]
        (TupleUnion<((u32, statics::Map<T, WrapOk<<T::Value as ValueTree>::Value,
                                                  <E::Value as ValueTree>::Value>>),
                     (u32, statics::Map<E, WrapErr<<T::Value as ValueTree>::Value,
                                                   <E::Value as ValueTree>::Value>>))>)
        -> MaybeErrValueTree<T::Value, E::Value>;
    /// `ValueTree` type corresponding to `MaybeErr`.
    #[derive(Clone, Debug)]
    pub struct MaybeErrValueTree[<T, E>][where T : ValueTree, E : ValueTree]
        (TupleUnionValueTree<(statics::Map<T, WrapOk<T::Value, E::Value>>,
                              Option<statics::Map<E, WrapErr<T::Value, E::Value>>>)>)
        -> Result<T::Value, E::Value>;
}

// These need to exist for the same reason as the one on `OptionStrategy`
impl<T : Strategy + fmt::Debug, E : Strategy + fmt::Debug> fmt::Debug
for MaybeOk<T, E> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "MaybeOk({:?})", self.0)
    }
}
impl<T : Strategy + fmt::Debug, E : Strategy + fmt::Debug> fmt::Debug
for MaybeErr<T, E> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "MaybeErr({:?})", self.0)
    }
}

/// Create a strategy for `Result`s where `Ok` values are taken from `t` and
/// `Err` values are taken from `e`.
///
/// `Ok` and `Err` are chosen with equal probability.
///
/// Generated values shrink to `Err`.
pub fn maybe_ok<T : Strategy, E : Strategy>(t: T, e: E) -> MaybeOk<T, E> {
    maybe_ok_weighted(0.5, t, e)
}

/// Create a strategy for `Result`s where `Ok` values are taken from `t` and
/// `Err` values are taken from `e`.
///
/// `probability_of_ok` is the probability (between 0.0 and 1.0, exclusive)
/// that `Ok` is initially chosen.
///
/// Generated values shrink to `Err`.
pub fn maybe_ok_weighted<T : Strategy, E : Strategy>(
    probability_of_ok: f64, t: T, e: E) -> MaybeOk<T, E>
{
    let (ok_weight, err_weight) = float_to_weight(probability_of_ok);

    MaybeOk(TupleUnion::new((
        (err_weight, statics::Map::new(e, WrapErr(PhantomData, PhantomData))),
        (ok_weight, statics::Map::new(t, WrapOk(PhantomData, PhantomData))),
    )))
}

/// Create a strategy for `Result`s where `Ok` values are taken from `t` and
/// `Err` values are taken from `e`.
///
/// `Ok` and `Err` are chosen with equal probability.
///
/// Generated values shrink to `Ok`.
pub fn maybe_err<T : Strategy, E : Strategy>(t: T, e: E) -> MaybeErr<T, E> {
    maybe_err_weighted(0.5, t, e)
}

/// Create a strategy for `Result`s where `Ok` values are taken from `t` and
/// `Err` values are taken from `e`.
///
/// `probability_of_ok` is the probability (between 0.0 and 1.0, exclusive)
/// that `Err` is initially chosen.
///
/// Generated values shrink to `Ok`.
pub fn maybe_err_weighted<T : Strategy, E : Strategy>(
    probability_of_err: f64, t: T, e: E) -> MaybeErr<T, E>
{
    let (err_weight, ok_weight) = float_to_weight(probability_of_err);

    MaybeErr(TupleUnion::new((
        (ok_weight, statics::Map::new(t, WrapOk(PhantomData, PhantomData))),
        (err_weight, statics::Map::new(e, WrapErr(PhantomData, PhantomData))),
    )))
}

#[cfg(test)]
mod test {
    use super::*;

    fn count_ok_of_1000<S : Strategy>(s: S) -> u32
    where S::Value : ValueTree<Value = Result<(), ()>> {
        let mut runner = TestRunner::new(Config::default());
        let mut count = 0;
        for _ in 0..1000 {
            count += s.new_value(&mut runner).unwrap()
                .current().is_ok() as u32;
        }

        count
    }

    #[test]
    fn probability_defaults_to_0p5() {
        let count = count_ok_of_1000(maybe_err(Just(()), Just(())));
        assert!(count > 450 && count < 550);
        let count = count_ok_of_1000(maybe_ok(Just(()), Just(())));
        assert!(count > 450 && count < 550);
    }

    #[test]
    fn probability_handled_correctly() {
        let count = count_ok_of_1000(maybe_err_weighted(
            0.1, Just(()), Just(())));
        assert!(count > 800 && count < 950);

        let count = count_ok_of_1000(maybe_err_weighted(
            0.9, Just(()), Just(())));
        assert!(count > 50 && count < 150);

        let count = count_ok_of_1000(maybe_ok_weighted(
            0.9, Just(()), Just(())));
        assert!(count > 800 && count < 950);

        let count = count_ok_of_1000(maybe_ok_weighted(
            0.1, Just(()), Just(())));
        assert!(count > 50 && count < 150);
    }

    #[test]
    fn shrink_to_correct_case() {
        let mut runner = TestRunner::new(Config::default());
        {
            let input = maybe_err(Just(()), Just(()));
            for _ in 0..64 {
                let mut val = input.new_value(&mut runner).unwrap();
                if val.current().is_ok() {
                    assert!(!val.simplify());
                    assert!(val.current().is_ok());
                } else {
                    assert!(val.simplify());
                    assert!(val.current().is_ok());
                }
            }
        }
        {
            let input = maybe_ok(Just(()), Just(()));
            for _ in 0..64 {
                let mut val = input.new_value(&mut runner).unwrap();
                if val.current().is_err() {
                    assert!(!val.simplify());
                    assert!(val.current().is_err());
                } else {
                    assert!(val.simplify());
                    assert!(val.current().is_err());
                }
            }
        }
    }
}
