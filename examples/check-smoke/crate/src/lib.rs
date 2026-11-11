#![no_std]
use vstd::prelude::*;

verus! {

pub open spec fn bounded(s: u64) -> bool {
    s < 8
}

pub fn smoke_entry(s: u64)
    requires
        bounded(s),
{
}

} // verus!
