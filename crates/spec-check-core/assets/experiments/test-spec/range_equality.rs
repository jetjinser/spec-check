use core::ops::Range;
use vstd::prelude::*;

verus! {

// Executable models of the three compiler-resolved standard equality bodies.
fn usize_eq(left: &usize, right: &usize) -> (equal: bool)
    ensures equal == (*left == *right),
{
    *left == *right
}

fn range_eq(left: &Range<usize>, right: &Range<usize>) -> (equal: bool)
    ensures equal == (*left == *right),
{
    usize_eq(&left.start, &right.start) && usize_eq(&left.end, &right.end)
}

fn option_eq(left: &Option<Range<usize>>, right: &Option<Range<usize>>) -> (equal: bool)
    ensures equal == (*left == *right),
{
    match (left, right) {
        (Some(l), Some(r)) => range_eq(l, r),
        (Some(_), None) => false,
        (None, Some(_)) => false,
        (None, None) => true,
    }
}

fn comparison_bridge(actual: &Option<Range<usize>>, expected: &Range<usize>) -> (equal: bool)
    ensures equal == (match *actual {
        Some(value) => value.start == expected.start && value.end == expected.end,
        None => false,
    }),
{
    let right = Some(Range { start: expected.start, end: expected.end });
    option_eq(actual, &right)
}

}
