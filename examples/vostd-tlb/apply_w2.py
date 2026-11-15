#!/usr/bin/env python3
"""W2: VmSpace::unmap without issuing or dispatching any TLB flush."""
import sys
p=sys.argv[1]+"/ostd/src/mm/vm_space.rs"
s=open(p).read()
a="""                    #[verus_spec(with Tracked(tlb_model))]
                    self.flusher.issue_tlb_flush_with(TlbFlushOp::Address(va), frame.into_dyn());"""
b="""                    #[verus_spec(with Tracked(tlb_model))]
                    self.flusher.issue_tlb_flush_with(TlbFlushOp::Range(va..va + len), pt);"""
c="""        #[verus_spec(with Tracked(tlb_model))]
        self.flusher.dispatch_tlb_flush();

        num_unmapped"""
for x in (a,b):
    assert s.count(x)==1,(x[:50],s.count(x))
s=s.replace(a,"                    let _ = (va, frame);")
s=s.replace(b,"                    let _ = (va, len, pt);")
assert s.count(c)==1, s.count(c)
s=s.replace(c,"        num_unmapped")
open(p,'w').write(s)
print("W2 applied")
