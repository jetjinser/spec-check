# Runtime consistency: ktest demands × postconditions (static classification)

Input: 153 ktest tests extracted from the frozen source; 22 non-blocked
candidates (8 direct, 10 helper-model, 4 quantified). For each candidate the
test's assertions are compared statically with the callee's effective
postcondition, using the census fact layer. Only K007 and K131 have executable
probe verdicts from M7b; the rest are classified from the contract text.

## Result

| classification | count | ids |
|---|---|---|
| run, `ObligationSupported` | 1 | K007 `MemoryRegionArray::push` |
| run, `ContractGapCandidate` | 1 | K131 `Mutex::try_lock` |
| static covered (ensures mention the asserted output; no probe run) | 14 | K014, K068, K069, K078, K080, K145–K153 |
| no effective postcondition | 1 | K075 `disable_preempt` |
| not expressible on the frozen surface | 5 | K070, K071, K072, K073, K012 |

Supported / Gap / 不可表达 therefore splits 1 / 1 / 20 when only executable
evidence is counted; 14 of the 20 have contract text that mentions the demanded
value but lack a model (iterator/RCU/tracked owner) to run the probe.

Notable individual readings:

- K078 `vm_space::query`: the ensures constrain both `Ok(Some)` and `Ok(None)`
  outcomes, unlike K131's Some-only contract. Statically covered.
- K080 `vm_space::find_next`: `res is Some ==> res->0 == final(self).0.va`
  covers the returned address on the success branch.
- K075 `disable_preempt`: returns a guard with no postcondition; the demand
  cannot be phrased against the contract.
- K072/K073 `page_walk`: not in the census verified surface (no VIR contract).
- K070/K071 `PageTable::cursor_mut`: requires tracked owners, guards and RCU
  state; no frozen-surface recipe.

## Boundary

Static mention is necessary, not sufficient: `static_covered` says the contract
mentions the state the test observes, not that it entails the assertion. No new
defect is claimed from this table. `report.py` regenerates the classification;
the per-candidate basis lives there and in the M7b archives.

## Why this is not a systematic runtime check

The remaining 131 candidates are blocked (117), excluded (14), or model-heavy.
Turning these into systematic Q-vs-runtime comparisons requires the missing
iterator/quantified/tracked-owner models; the current number is a coverage
statement, not a property of the crate.
