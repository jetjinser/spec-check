// Observable reads and assertion attempts, including partial executions.
enum ReadOutcome<T> { Returned(Option<T>), Panicked, Stalled }
enum CheckEvent<T> { Read(ReadOutcome<T>), Some(T), None }

spec fn foreach_steps<T>(expected: Seq<T>, reads: Seq<ReadOutcome<T>>) -> Seq<CheckEvent<T>>
    decreases reads.len(),
{
    if reads.len() == 0 { Seq::empty() } else {
        let events = seq![CheckEvent::Read(reads[0])];
        match reads[0] {
            ReadOutcome::Returned(value) => {
                if expected.len() == 0 { events.push(CheckEvent::None) } else {
                    let checked = events.push(CheckEvent::Some(expected[0]));
                    if value == Some(expected[0]) {
                        checked + foreach_steps(expected.drop_first(), reads.drop_first())
                    } else { checked }
                }
            },
            _ => events,
        }
    }
}

spec fn indexed_steps<T>(expected: Seq<T>, i: int, reads: Seq<ReadOutcome<T>>) -> Seq<CheckEvent<T>>
    recommends 0 <= i <= expected.len(),
    decreases reads.len(),
{
    if reads.len() == 0 { Seq::empty() } else {
        let events = seq![CheckEvent::Read(reads[0])];
        match reads[0] {
            ReadOutcome::Returned(value) => {
                if i == expected.len() { events.push(CheckEvent::None) } else {
                    let checked = events.push(CheckEvent::Some(expected[i]));
                    if value == Some(expected[i]) {
                        checked + indexed_steps(expected, i + 1, reads.drop_first())
                    } else { checked }
                }
            },
            _ => events,
        }
    }
}

proof fn ordered_checks_correspond<T>(expected: Seq<T>, i: int, reads: Seq<ReadOutcome<T>>)
    requires 0 <= i <= expected.len(),
    ensures foreach_steps(expected.subrange(i, expected.len() as int), reads)
        == indexed_steps(expected, i, reads),
    decreases reads.len(),
{
    if reads.len() > 0 && i < expected.len() {
        let tail = expected.subrange(i, expected.len() as int);
        assert(tail[0] == expected[i]);
        assert(tail.drop_first() =~= expected.subrange(i + 1, expected.len() as int));
        if reads[0] matches ReadOutcome::Returned(value) && value == Some(expected[i]) {
            ordered_checks_correspond(expected, i + 1, reads.drop_first());
        }
    }
}

proof fn range_comparison_corresponds(actual: Option<Range<usize>>, expected: Range<usize>)
    ensures (actual == Some(expected)) == (match actual {
        Some(value) => value.start == expected.start && value.end == expected.end,
        None => false,
    }),
{
}
