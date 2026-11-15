import sys
root=sys.argv[1]
M=[
 ("ostd/src/boot/memory_region.rs",
  "        requires\n            base + len <= MAX_PADDR,\n        returns",
  "        requires\n            true,\n        returns"),
 ("ostd/src/mm/kspace/mod.rs",
  "    requires\n        pa + LINEAR_MAPPING_BASE_VADDR < usize::MAX,\n    returns",
  "    requires\n        true,\n    returns"),
 ("ostd/src/mm/mod.rs",
  "    requires\n        1 <= level <= C::NR_LEVELS() + 1,\n    ensures",
  "    requires\n        true,\n    ensures"),
 ("ostd/src/cpu/set.rs",
  "        requires\n            ordering != Ordering::AcqRel,\n        ensures",
  "        requires\n            true,\n        ensures"),
 ("ostd/src/cpu/set.rs",
  "        requires\n            ordering != Ordering::Acquire,\n            ordering != Ordering::AcqRel,\n    )]",
  "        requires\n            true,\n    )]"),
 ("ostd/src/mm/io.rs",
  "        requires\n            old(self).inv(),\n            nbytes <= old(self).remain_spec(),\n        ensures",
  "        requires\n            true,\n        ensures"),
 ("ostd/src/mm/io.rs",
  "        requires\n            old(self).inv(),\n            nbytes <= old(self).avail_spec(),\n        ensures",
  "        requires\n            true,\n        ensures"),
]
for f,old,new in M:
    p=root+"/"+f
    s=open(p).read()
    n=s.count(old)
    assert n==1, (f,n,old[:40])
    open(p,'w').write(s.replace(old,new))
print("mutated",len(M))
