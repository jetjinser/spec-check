// The source moves the next expected range before calling next(). The indexed
// form may read its fields later. Expected reads are pure under the instance gate.
enum EvalEvent<T> { Expected(T), Observed(CheckEvent<T>) }

spec fn source_evaluations<T>(expected: Seq<T>, reads: Seq<ReadOutcome<T>>) -> Seq<EvalEvent<T>>
    decreases reads.len(),
{
    if reads.len() == 0 { Seq::empty() } else {
        let prefix = if expected.len() == 0 { Seq::empty() }
            else { seq![EvalEvent::Expected(expected[0])] };
        let events = prefix.push(EvalEvent::Observed(CheckEvent::Read(reads[0])));
        match reads[0] {
            ReadOutcome::Returned(value) => {
                if expected.len() == 0 {
                    events.push(EvalEvent::Observed(CheckEvent::None))
                } else {
                    let checked = events.push(EvalEvent::Observed(CheckEvent::Some(expected[0])));
                    if value == Some(expected[0]) {
                        checked + source_evaluations(expected.drop_first(), reads.drop_first())
                    } else { checked }
                }
            },
            _ => events,
        }
    }
}

spec fn visible_evaluations<T>(events: Seq<EvalEvent<T>>) -> Seq<CheckEvent<T>>
    decreases events.len(),
{
    if events.len() == 0 { Seq::empty() } else {
        match events[0] {
            EvalEvent::Expected(_) => visible_evaluations(events.drop_first()),
            EvalEvent::Observed(event) => seq![event] + visible_evaluations(events.drop_first()),
        }
    }
}

proof fn visible_concat<T>(a: Seq<EvalEvent<T>>, b: Seq<EvalEvent<T>>)
    ensures visible_evaluations(a + b) == visible_evaluations(a) + visible_evaluations(b),
    decreases a.len(),
{
    if a.len() > 0 {
        assert((a + b).drop_first() =~= a.drop_first() + b);
        visible_concat(a.drop_first(), b);
    }
}

proof fn expected_reads_are_unobservable<T>(expected: Seq<T>, reads: Seq<ReadOutcome<T>>)
    ensures visible_evaluations(source_evaluations(expected, reads)) == foreach_steps(expected, reads),
    decreases reads.len(),
{
    reveal_with_fuel(visible_evaluations, 4);
    if reads.len() > 0 && expected.len() > 0 {
        if reads[0] matches ReadOutcome::Returned(value) && value == Some(expected[0]) {
            expected_reads_are_unobservable(expected.drop_first(), reads.drop_first());
            let prefix = seq![EvalEvent::Expected(expected[0]),
                EvalEvent::Observed(CheckEvent::Read(reads[0])),
                EvalEvent::Observed(CheckEvent::Some(expected[0]))];
            let tail = source_evaluations(expected.drop_first(), reads.drop_first());
            assert(source_evaluations(expected, reads) =~= prefix + tail);
            assert(visible_evaluations(prefix) =~= seq![CheckEvent::Read(reads[0]), CheckEvent::Some(expected[0])]);
            visible_concat(prefix, tail);
        }
    }
}

proof fn effectful_evaluations_correspond<T>(expected: Seq<T>, reads: Seq<ReadOutcome<T>>)
    ensures visible_evaluations(source_evaluations(expected, reads)) == indexed_steps(expected, 0, reads),
{
    expected_reads_are_unobservable(expected, reads);
    ordered_checks_correspond(expected, 0, reads);
    assert(expected.subrange(0, expected.len() as int) =~= expected);
}
