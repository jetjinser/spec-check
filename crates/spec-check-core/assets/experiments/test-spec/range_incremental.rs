// Each assertion stays immediately after its corresponding next call.
fn consume_incrementally<I: Iterator<Item = Range<usize>> + IteratorSpec, const N: usize>(
    mut res: I,
    expected: [Range<usize>; N],
)
    requires
        res.obeys_prophetic_iter_laws(),
        res.remaining().is_prefix_of(expected@),
        res.will_return_none() ==> res.remaining() == expected@,
{
    let mut i: usize = 0;
    while i < N
        invariant
            i <= N,
            res.obeys_prophetic_iter_laws(),
            res.remaining().is_prefix_of(expected@.subrange(i as int, N as int)),
            res.will_return_none()
                ==> res.remaining() == expected@.subrange(i as int, N as int),
        decreases N - i,
    {
        let value = res.next();
        let matches = match value {
            Some(actual) => actual.start == expected[i].start && actual.end == expected[i].end,
            None => false,
        };
        assert!(matches);
        i += 1;
    }
    let last = res.next();
    assert!(last.is_none());
}
