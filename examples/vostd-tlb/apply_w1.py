#!/usr/bin/env python3
"""W1: VmSpace::map without TLB flush on replacement."""
import sys
p=sys.argv[1]+"/ostd/src/mm/vm_space.rs"
s=open(p).read()
old="""                //debug_assert_eq!(va, start_va);
                let old_frame = item.frame;

                #[verus_spec(with Tracked(tlb_model))]
                self.flusher.issue_tlb_flush_with(
                    TlbFlushOp::Address(start_va),
                    old_frame.into_dyn(),
                );
                #[verus_spec(with Tracked(tlb_model))]
                self.flusher.dispatch_tlb_flush();"""
new="""                //debug_assert_eq!(va, start_va);
                let _ = (va, item);"""
assert s.count(old)==1, s.count(old)
open(p,'w').write(s.replace(old,new))
print("W1 applied")
