import json, re, os
from collections import Counter, OrderedDict

D=json.load(open('/tmp/m7c-candidates/_parsed3.json'))
secs={s['name']:s for s in D['secs']}
rows=[r for r in D['rows'] if r['n']>0]

ROOT='/tmp/vostd-43da079-ktest-chan/'
FROZEN_CACHE={}
def frozen_has(row):
    f=row['loc'].rsplit(':',1)[0]
    if f not in FROZEN_CACHE:
        p=ROOT+f
        FROZEN_CACHE[f]=open(p,errors='replace').read() if os.path.exists(p) else ''
    txt=FROZEN_CACHE[f]
    meth=row['name'].split('::')[-1]
    if re.search(r'\bfn\s+'+re.escape(meth)+r'\b',txt): return True
    # derived / static / macro
    return False

# (family, ctor, expected, draft, evidence, note, registry)
# family may be None -> chosen from bullet kind (final-relation for &mut, else default)
VR='value-readback'; GI='guard-identity'; IN='is_none-demand'; FR='final-relation'; OT='other'
C='constructible'; N='needs-recipe'; B='blocked'
GAP='gap'; SUP='supported'; INC='inconclusive'

def d_lock_new(t,ctor,val='5'):
    return f"let l = {ctor}::new({val});\nlet g = l.read();\nassert(g.view() == {val});\nlet _held = g;"
def d_guard_view(t,ctorm,val='5'):
    return f"let l = {ctorm}::new({val});\nlet g = l.read();\nassert(g.view() == {val});\nlet _held = g;"
def d_arc(expr):
    return f"let a = RwArc::new(5u64);\n{expr}\nlet _held = g;"
def d_vmreader(meth,arg):
    return f"let bytes = [1u8,2,3,4,5,6,7,8];\nlet mut r = VmReader::from(&bytes[..]);\nlet _ = r.{meth}({arg});\nassert(r.remain() <= {arg});  // needs final(r) link"

# default common drafts
DRAFT={
 'vr_guard': "let l = {C}::new(5);\nlet g = l.{M}();\nassert(g.view() == 5);\nlet _held = g;",
 'vr_lock_new': "let l = {C}::new(5);\nlet g = l.read();\nassert(g.view() == 5);\nlet _held = g;",
 'vr_new_read': "let l = {C}::new(5);\nlet g = l.read();\nassert(g.view() == 5);\nlet _held = g;",
 'vr_get_cloned': "let a = RwArc::new(5u64);\nlet v = a.get_cloned();\nassert(v == 5);",
 'vr_clone_ro': "let a = RwArc::new(5u64);\nlet ro = a.clone_ro();\nlet g = ro.read();\nassert(g.view() == 5);\nlet _held = g;",
 'vr_upgrade': "let l = {C}::new(5);\nlet u = l.upread();\nlet g = u.{M}();\nassert(g.view() == 5);\nlet _held = g;",
 'fr_vmreader': "let bytes = [1u8,2,3,4,5,6,7,8];\nlet mut r = VmReader::from(&bytes[..]);\nlet _ = r.{M}(4);\nassert(final(r).remain() <= 4);",
 'fr_vmwriter': "let mut buf = [0u8; 8];\nlet mut w = VmWriter::from(&mut buf[..]);\nlet _ = w.{M}(4);\nassert(final(w).remain_spec() <= 4);",
 'fr_vmwriter_from': "let mut buf = [0u8; 8];\nlet w = VmWriter::from(&mut buf[..]);\nassert(final(buf).len() == old(buf).len());",
 'vr_vmreader_cursor': "let bytes = [1u8,2,3];\nlet r = VmReader::from(&bytes[..]);\nassert(r.cursor() == bytes.as_virt_ptr());",
 'demand_vmreader_has_remain': "let bytes = [1u8,2,3];\nlet r = VmReader::from(&bytes[..]);\nassert(r.has_remain());",
 'demand_vmwriter_has_avail': "let mut buf = [0u8; 3];\nlet w = VmWriter::from(&mut buf[..]);\nassert(w.has_avail());",
 'vr_either_side': "let e = Either::<u64,u64>::Left(5);\nassert(e.{M}() == Some(5));",
 'demand_either_side': "let e = Either::<u64,u64>::Left(5);\nassert(e.{M}());",
 'vr_either_clone': "let e = Either::<u64,u64>::Left(5);\nassert(e.clone() == e);",
 'vr_atomic_cpuset': "let cs = CpuSet::new_empty();\nlet a = AtomicCpuSet::new(cs);\nassert(a.load(Ordering::Relaxed) == cs);",
 'demand_rcuoption': "let rcu = RcuOption::<Box<u64>>::new(Some(Box::new(5u64)));\nlet g = rcu.read();\nassert(!g.is_none());\nlet _held = g;",
 'vr_rcuoption_none': "let rcu = RcuOption::<Box<u64>>::new_none();\nlet g = rcu.read();\nassert(g.is_none());\nlet _held = g;",
 'vr_linkedlist': "let l = LinkedList::<M>::new();\nassert(l.is_empty());",
 'vr_link': "let l = Link::<M>::new(meta);\nassert(*(l.deref()) == meta);",
 'vr_drhd': "let d = Drhd::from_bytes(&hdr_bytes);  // requires valid header\nassert(d.register_base_addr() == expected);",
 'vr_wait': "let q = WaitQueue::new();\nassert(q.{M}() == {V});",
 'vr_is_page_aligned': "assert(is_page_aligned(4096));\nassert(!is_page_aligned(1));",
 'vr_rangealloc': "let r = RangeAllocator::new(0x1000..0x2000);\nassert(r == RangeAllocator);  // ZST, vacuous",
 'vr_jiffies': "let j = Jiffies::elapsed();\nassert(j.as_u64() == ELAPSED.load(Ordering::Relaxed));",
 'gi_guardian': "let g = {T}::guard();\nassert(g.rel_guardian({T}));  // no such predicate in frozen face",
 'gi_transfer': "let mut g = disable_preempt();\nlet g2 = g.transfer_to();\nassert(g2.mode() == old(g).mode());  // no accessor",
 'gi_rcu_read': "let rcu = Rcu::<Box<u64>>::new(Box::new(5));\nlet g = rcu.read();\nassert(g.rel_rcu(rcu));",
 'fr_framealloc': "let mut o = FrameAllocOptions::new();\no.zeroed(false);\nassert(final(o).zeroed == false);  // field private, no accessor",
 'vr_framelloc_new': "let o = FrameAllocOptions::new();\nassert(o.zeroed == true);  // field private, no accessor",
 'fr_vmio_owner': "let mut reader = VmReader::from(&bytes[..]);\nlet _ = reader.read_bytes(0, &mut buf, Tracked(&mut owner));\nassert(final(owner).mem_view is Some);",
 'fr_vmio_slice': "let mut reader = VmReader::from(&bytes[..]);\nlet _ = reader.read_slice(0, &mut out, Tracked(&mut owner));\nassert(final(out).len() == old(out).len());",
 'other_result': "let r = {EXPR};\nassert(r is Ok);  // error/side-effect semantics not specified",
 'other_dbg': "let s = format!(\"{:?}\", x);\nassert(s == expected_string);  // fmt output unmodeled",
 'other_ghost': "// proof-only / ghost state; no exec obligation client",
 'other_blocked': "// no expressible/constructible obligation in frozen face",
}

# ---- per-row assignment: idx -> (default_family, ctor, expected, draft, evidence, note, registry)
A={}
A2={}
def put(i,fam,ctor,exp,draft,ev,note,reg='none'):
    A[i]=(fam,ctor,exp,draft,ev,note,reg)

put(1,OT,B,INC,'other_ghost','ostd/specs/mm/io.rs:257','proof-only ghost return VmIoMemView; no exec client')
put(2,FR,B,INC,'other_ghost','ostd/specs/mm/vm_space.rs:335','proof-only ghost &mut self')
put(3,FR,B,INC,'other_ghost','ostd/specs/mm/vm_space.rs:410','proof-only ghost &mut self')
put(4,VR,B,INC,'other_blocked','ostd/src/arch/x86/kernel/acpi/dmar.rs:92','external_body; hardware discovery; Option<Dmar> no observable value')
for i in range(5,13):
    put(i,OT,B,INC,'other_dbg','ostd/src/arch/x86/kernel/acpi/remapping.rs:35','derive(Clone); no PartialEq/View on these ACPI structs; private fields')
put(6,VR,N,GAP,'vr_drhd','ostd/src/arch/x86/kernel/acpi/remapping.rs:44,349','from_bytes is pub with satisfiable requires; register_base_addr pub; recipe = valid header bytes')
put(13,VR,B,INC,'other_blocked','ostd/src/arch/x86/kernel/apic/mod.rs:113','receiver requires XApic/X2Apic instance (hardware); trait decl')
put(14,VR,B,INC,'other_blocked','ostd/src/arch/x86/kernel/apic/mod.rs:115','same')
put(15,VR,B,INC,'other_blocked','ostd/src/arch/x86/kernel/apic/mod.rs:130','same')
put(16,VR,B,INC,'other_blocked','ostd/src/arch/x86/kernel/apic/x2apic.rs:21','pub(crate) callable, but returned X2Apic has only private unit field; no spec observable')
put(17,FR,B,INC,'other_blocked','ostd/src/arch/x86/kernel/apic/x2apic.rs:34','hardware MSR writes; no observable state')
for i in range(18,21):
    put(i,VR,B,INC,'other_blocked','ostd/src/arch/x86/kernel/apic/x2apic.rs:78','hardware instance unobtainable in obligation client')
put(21,VR,B,INC,'other_blocked','ostd/src/arch/x86/kernel/apic/xapic.rs:92','private fn')
put(22,FR,B,INC,'other_blocked','ostd/src/arch/x86/kernel/apic/xapic.rs:52','hardware MMIO; no observable state')
for i in range(23,26):
    put(i,VR,B,INC,'other_blocked','ostd/src/arch/x86/kernel/apic/xapic.rs:161','hardware instance')
put(26,VR,B,INC,'other_blocked','ostd/src/arch/x86/kernel/apic/xapic.rs:246','private fn')
put(27,OT,B,INC,'other_dbg','ostd/src/arch/x86/mm/mod.rs:63','derive(Clone); config ZST with no equality observable')
put(28,OT,B,INC,'other_dbg','ostd/src/cpu/set.rs:24','derive(Clone); CpuSet has no PartialEq/View at spec level')
put(29,VR,C,GAP,'vr_atomic_cpuset','ostd/src/cpu/set.rs:537,145,558','AtomicCpuSet::new pub, requires CpuSet.inv (new_empty satisfies); load pub; contract only gives num_parts', 'partial:F-010')
for i in range(30,34):
    put(i,VR,N,GAP,'vr_dma_coherent','ostd/src/mm/dma/dma_coherent.rs:367','DmaCoherent::map(Segment) pub but requires segment.inv(); Segment recipe needed', 'partial:F-007')
put(34,VR,N,GAP,'vr_dma_coherent','ostd/src/mm/dma/dma_coherent.rs:367','trait impl forwarding; same recipe', 'partial:F-007')
put(35,VR,N,GAP,'vr_dma_coherent','ostd/src/mm/dma/dma_coherent.rs:380','trait impl forwarding; same recipe', 'partial:F-007')
put(36,OT,N,INC,'other_result','ostd/src/mm/dma/dma_coherent.rs:547','VmIoOnce for DmaCoherent; Result error semantics; Segment recipe', 'partial:F-007')
put(37,OT,N,INC,'other_result','ostd/src/mm/dma/dma_coherent.rs:547','same', 'partial:F-007')
put(38,VR,N,GAP,'vr_dma_slice','ostd/src/mm/dma/dma_stream.rs:125,136','DmaStreamSlice is pub with pub fields; requires a Dma: AsRef<DmaStream> instance', 'partial:F-008')
put(39,OT,N,INC,'other_result','ostd/src/mm/dma/dma_stream.rs:125','external_body; Result semantics', 'partial:F-008')
put(40,VR,N,GAP,'vr_dma_slice','ostd/src/mm/dma/dma_stream.rs:169','external_body; slice constructible but DmaStream recipe needed', 'partial:F-008')
for i,(ev,reg) in zip(range(41,47),[('ostd/src/mm/dma/dma_stream.rs:738','partial:F-008'),('ostd/src/mm/dma/dma_stream.rs:759','partial:F-008'),('ostd/src/mm/dma/dma_stream.rs:711','partial:F-008'),('ostd/src/mm/dma/dma_stream.rs:815','partial:F-008'),('ostd/src/mm/dma/dma_stream.rs:837','partial:F-008'),('ostd/src/mm/dma/dma_stream.rs:850','partial:F-008')]):
    put(i,VR,N,GAP,'vr_dma_stream','ostd/src/mm/dma/dma_stream.rs:94','DmaStream.inner is pub RwArc but no pub DmaStream constructor; needs inner AtomicDataWithOwner recipe',reg)
put(47,VR,N,GAP,'vr_dma_trait','ostd/src/mm/dma/mod.rs:97','trait declaration; concrete impls need DmaCoherent/DmaStream recipe')
put(48,OT,B,INC,'other_blocked','ostd/src/mm/dma/mod.rs:111','private fn')
put(49,OT,B,INC,'other_blocked','ostd/src/mm/dma/mod.rs:141','external_body; closure-based dma_type')
put(50,OT,B,INC,'vr_framelloc_new','ostd/src/mm/frame/allocator.rs:20','FrameAllocOptions.zeroed is private with no accessor; Default findable but unobservable')
put(51,VR,B,INC,'vr_framelloc_new','ostd/src/mm/frame/allocator.rs:35','field zeroed private; no accessor')
put(52,FR,B,INC,'fr_framealloc','ostd/src/mm/frame/allocator.rs:46','state change unobservable (private field, no accessor)')
put(53,VR,N,GAP,'vr_linkedlist','ostd/src/mm/frame/linked_list.rs:195,233','LinkedList::new pub const fn; is_empty pub; needs concrete M: AnyFrameMeta+Repr<MetaSlotSmall>')
put(54,VR,N,GAP,'vr_linkedlist','ostd/src/mm/frame/linked_list.rs:200,233','Default delegates to new; same M recipe')
put(55,GI,N,INC,'vr_cursor_as_list','ostd/src/mm/frame/linked_list.rs:575,1474','needs CursorMut from cursor_front_mut; returned &LinkedList identity has no spec predicate')
put(56,VR,N,GAP,'vr_link','ostd/src/mm/frame/linked_list.rs:1680,1696','Link::new pub; Deref returns &M without ensures; needs M relation/PartialEq')
put(57,VR,N,GAP,'vr_link','ostd/src/mm/frame/linked_list.rs:1688','DerefMut return &mut M; Link::new+meta needed')
A2[57]=[(VR,N,GAP,'vr_link','ostd/src/mm/frame/linked_list.rs:1688','DerefMut return &mut M; Link::new+meta needed'),(FR,N,GAP,'fr_derefmut_self','ostd/src/mm/frame/linked_list.rs:1688','final(self) unmentioned; needs Link recipe')]
put(58,VR,N,GAP,'vr_link','ostd/src/mm/frame/linked_list.rs:1696','Link::new pub const fn(meta); concrete M recipe')
put(59,OT,B,INC,'other_blocked','ostd/src/mm/frame/linked_list.rs:1712','&mut Self ghost trait impl; no observable')
put(60,IN,N,INC,'demand_link_is_untyped','ostd/src/mm/frame/linked_list.rs:1721','bool post unmentioned; no spec fn relating Link meta to is_untyped')
put(61,OT,B,INC,'other_blocked','ostd/src/mm/frame/meta.rs:277','generic trait declaration; &mut Self')
put(62,IN,B,INC,'other_blocked','ostd/src/mm/frame/meta.rs:277','generic trait declaration; relation needs spec fn')
put(63,OT,B,INC,'other_dbg','ostd/src/mm/frame/mod.rs:153','Formatter effect unmodeled')
put(64,OT,B,INC,'other_blocked','ostd/src/mm/frame/mod.rs:879','external_body; requires meta region state')
put(65,OT,B,INC,'other_dbg','ostd/src/mm/frame/segment.rs:63','Formatter effect unmodeled')
put(66,FR,N,INC,'fr_segment_repr','ostd/src/mm/frame/segment.rs:278','ghost repr_perm &mut M::ReprPerm; needs MetaRegionOwners/repr perms recipe')
put(67,OT,B,INC,'other_dbg','ostd/src/mm/frame/unique.rs:60','Formatter effect unmodeled')
put(68,OT,B,INC,'other_blocked','ostd/src/mm/frame/unique.rs','private drop fn')
put(69,VR,N,GAP,'vr_unique_from_shared','ostd/src/mm/frame/unique.rs:613,98','UniqueFrame::from_unused pub; needs frame metadata region recipe')
put(70,VR,N,GAP,'vr_untyped_reader','ostd/src/mm/frame/untyped.rs:24','UntypedMem::reader pub but UntypedMem needs untyped frame recipe')
put(71,VR,N,GAP,'vr_untyped_writer','ostd/src/mm/frame/untyped.rs:24','same')
put(72,OT,B,INC,'other_dbg','ostd/src/mm/io.rs:653','derive(Clone); artifact renders wrong type (Drhd) for grouped Clone')
# VmIo 73-79: per-bullet assignment
VIO_RET=('let r = reader.read_bytes(0, &mut buf, Tracked(&mut owner));\nassert(r is Ok);','ostd/src/mm/io.rs:1309','Result + ghost owner; VmIo impl only for DmaStream/DmaCoherent (dma_stream.rs:1054, dma_coherent.rs:547); needs DMA recipe')
A2[73]=[(OT,N,INC,'other_result','ostd/src/mm/io.rs:1309','Result<(),Error> + ghost owner; only DMA impls; needs recipe'),
        (FR,N,GAP,'fr_vmio_slice','ostd/src/mm/io.rs:1309','&mut [u8]; only DMA impls; needs recipe'),
        (FR,N,GAP,'fr_vmio_owner','ostd/src/mm/io.rs:1309','ghost &mut P owner; only DMA impls; needs recipe')]
A2[74]=[(VR,N,GAP,'let v = reader.read_val::<u64>(0, Tracked(&mut owner));\nassert(v is Ok ==> v.unwrap() == expected);','ostd/src/mm/io.rs:1324','value readback; only DMA impls'),
        (FR,N,GAP,'fr_vmio_owner','ostd/src/mm/io.rs:1324','ghost owner')]
A2[75]=[(OT,N,INC,'other_result','ostd/src/mm/io.rs:1336','Result + ghost owner'),
        (FR,N,GAP,'fr_vmio_slice','ostd/src/mm/io.rs:1336','&mut [T] slice'),
        (FR,N,GAP,'fr_vmio_owner','ostd/src/mm/io.rs:1336','ghost owner')]
for i,ln in [(76,1359),(77,1375),(78,1384)]:
    A2[i]=[(OT,N,INC,'other_result',f'ostd/src/mm/io.rs:{ln}','Result<(),Error>; only DMA impls'),
           (FR,N,GAP,'fr_vmio_owner',f'ostd/src/mm/io.rs:{ln}','ghost &mut P owner')]
A2[79]=[(VR,N,GAP,'let n = writer.write_vals(...);\nassert(n == expected_len);','ostd/src/mm/io.rs:1421','Result<usize> value readback'),
        (FR,N,GAP,'fr_vmio_owner','ostd/src/mm/io.rs:1421','ghost owner')]
put(80,VR,N,GAP,'let v = dma.read_once::<u64>(0);\nassert(v is Ok ==> v.unwrap() == written);','ostd/src/mm/io.rs:1503','VmIoOnce for DmaCoherent; needs DMA recipe')
put(81,OT,N,INC,'other_result','ostd/src/mm/io.rs:1510','Result<(),Error>; needs DMA recipe')
put(82,VR,C,GAP,'vr_vmreader_cursor','ostd/src/mm/io.rs:1580,2016','VmReader::from(&[u8]) pub; struct fields pub')
put(83,IN,C,GAP,'demand_vmreader_has_remain','ostd/src/mm/io.rs:1589,2016','from(&[u8]) pub; has_remain has only requires self.inv')
put(84,FR,C,GAP,'fr_vmreader','ostd/src/mm/io.rs:1606,2016','limit ensures r.* but no final(self); from pub')
put(85,FR,C,GAP,'fr_vmreader','ostd/src/mm/io.rs:1631,2016','skip ensures r.* but no final(self); requires nbytes<=remain satisfiable')
put(86,VR,C,GAP,'let mut buf=[0u8;3];\nlet w=VmWriter::from(&mut buf[..]);\nassert(w.cursor() == buf.as_virt_ptr());','ostd/src/mm/io.rs:1714,1959','from(&mut [u8]) pub')
put(87,IN,C,GAP,'demand_vmwriter_has_avail','ostd/src/mm/io.rs:1723,1959','from(&mut [u8]) pub')
put(88,FR,C,GAP,'fr_vmwriter','ostd/src/mm/io.rs:1740,1959','limit ensures r.* but no final(self)')
put(89,FR,C,GAP,'fr_vmwriter','ostd/src/mm/io.rs:1765,1959','skip ensures r.* but no final(self)')
put(90,VR,N,GAP,'let p = slice.as_virt_ptr();\nassert(p.vaddr == slice.as_ptr() as usize);','ostd/src/mm/io.rs','trait AsVirtPtr; needs a pointer/slice recipe')
put(91,FR,C,GAP,'fr_vmwriter_from','ostd/src/mm/io.rs:1959','from ensures owner/range but not final(slice)')
put(92,OT,B,INC,'other_blocked','ostd/src/mm/io.rs:pod_once_impls','private fn')
put(93,VR,C,GAP,'vr_rangealloc','ostd/src/mm/kspace/kvirt_area.rs:110,123','ZST with pub const new; obligation vacuous (no state)')
put(94,OT,B,INC,'other_blocked','ostd/src/mm/kspace/kvirt_area.rs:143','private exec static')
put(95,FR,B,INC,'other_blocked','ostd/src/mm/kspace/kvirt_area.rs:506','ghost entry_owners/guards; KVirtArea has no public constructor (allocator static private)')
put(96,FR,B,INC,'other_blocked','ostd/src/mm/kspace/kvirt_area.rs','ghost guards; same')
put(97,OT,B,INC,'other_blocked','ostd/src/mm/kspace/mod.rs:163','global OnceImpl static; no accessor/relation')
put(98,OT,B,INC,'other_dbg','ostd/src/mm/kspace/mod.rs:155','derive(Clone); config ZST')
put(99,VR,N,GAP,'vr_dma_trait','ostd/src/mm/mod.rs:97','trait declaration; concrete receiver recipe')
put(100,VR,C,GAP,'vr_is_page_aligned','ostd/src/mm/mod.rs:323','pub const fn pure; literals suffice; no contract at all')
put(101,FR,B,INC,'other_blocked','ostd/src/mm/page_table/cursor/locking.rs:265','external_body; &mut Cursor, page-table state recipe')
put(102,FR,B,INC,'other_blocked','ostd/src/mm/page_table/cursor/locking.rs:632','unsafe private fn; ghost guards')
put(103,VR,N,GAP,'let r = cursor.protect_next(len);\nassert(r == Some(expected_range));','ostd/src/mm/page_table/cursor/mod.rs','CursorMut from page-table recipe; Option<Range> observable')
put(104,VR,B,INC,'other_blocked','ostd/src/mm/page_table/mod.rs:628','external_body; generic ? return type; no accessor')
put(105,VR,N,INC,'let pt = PageTable::<KernelPtConfig>::new_kernel_page_table();\nassert(pt.root_paddr() == expected);','ostd/src/mm/page_table/mod.rs:918','pub(crate) callable from crate probe but external_body; fresh-root paddr not specified')
A2[106]=[(VR,B,INC,'other_blocked','ostd/src/mm/page_table/mod.rs:995','pub(in crate::mm) not callable from crate-root probe'),
         (FR,B,INC,'other_blocked','ostd/src/mm/page_table/mod.rs:995','ghost guards; same visibility')]
put(107,VR,B,INC,'other_blocked','ostd/src/mm/page_table/mod.rs:1288','external_body; generic C; no page-table state contract')
put(108,OT,B,INC,'other_blocked','ostd/src/mm/page_table/mod.rs:1761','external_body trait method; Self unconstrained')
put(109,OT,B,INC,'other_blocked','ostd/src/mm/page_table/node/mod.rs:143','AnyFrameMeta impl; &mut Self')
put(110,IN,B,INC,'other_blocked','ostd/src/mm/page_table/node/mod.rs:187','generic impl needs page meta instance')
put(111,VR,N,GAP,'let m = PageTablePageMeta::<C>::new(0);\nassert(m.level_spec() == 0);','ostd/src/mm/page_table/node/mod.rs:631','pub new(level) but needs PageTableConfig C and page-meta storage recipe')
put(112,VR,B,INC,'other_blocked','ostd/src/mm/tlb.rs:48','TlbFlusher has no public accessor for its state')
for i in range(113,117):
    put(i,FR,B,INC,'other_blocked','ostd/src/mm/tlb.rs:69','client constructible via new(&AtomicCpuSet) but final state unobservable (private fields)')
put(117,OT,B,INC,'other_dbg','ostd/src/mm/tlb.rs:191','derive(Clone); TlbFlushOp equality not exposed in spec')
put(118,VR,B,INC,'other_blocked','ostd/src/mm/tlb.rs:278','private type/fn')
put(119,IN,B,INC,'other_blocked','ostd/src/mm/tlb.rs','private fn')
for i in range(120,124):
    put(i,FR,B,INC,'other_blocked','ostd/src/mm/tlb.rs','private OpsStack &mut self')
A2[124]=[(VR,N,GAP,'let s = VmSpace::default();\nlet c = s.cursor(&guard, &va);\nassert(c is Ok);','ostd/src/mm/vm_space.rs:163','needs tracked kernel_owner_opt + ghost regions/guards recipe'),
         (FR,N,INC,'other_blocked','ostd/src/mm/vm_space.rs:163','ghost &mut MetaRegionOwners'),
         (FR,N,INC,'other_blocked','ostd/src/mm/vm_space.rs:163','ghost &mut Guards')]
A2[125]=[(VR,N,GAP,'let s = VmSpace::new();\nlet c = s.cursor_mut(&guard, &va);\nassert(c is Ok);','ostd/src/mm/vm_space.rs:207','pub but requires tracked kernel_owner_opt / ghost page-table regions'),
         (FR,N,INC,'other_blocked','ostd/src/mm/vm_space.rs:207','ghost &mut Guards')]
for i in (126,127):
    A2[i]=[(FR,N,INC,'other_blocked','ostd/src/mm/vm_space.rs:306','ghost &mut MetaRegionOwners; page-table recipe'),
           (FR,N,INC,'other_blocked','ostd/src/mm/vm_space.rs:306','ghost &mut Guards')]
put(128,FR,B,INC,'other_blocked','ostd/src/mm/vm_space.rs','ghost &mut TlbModel; page-table cursor recipe')
put(129,VR,N,GAP,'let r = cursor.protect_next(len);\nassert(r == Some(expected_range));','ostd/src/mm/vm_space.rs','CursorMut recipe; Option<Range> observable')
put(130,OT,B,INC,'other_dbg','ostd/src/mm/vm_space.rs:1580','derive(Clone); config ZST')
put(131,GI,B,INC,'gi_guardian','ostd/src/sync/guard.rs:37','no relation predicate/accessor for DisabledPreemptGuard')
put(132,GI,B,INC,'gi_guardian','ostd/src/sync/guard.rs:39','same')
A2[133]=[(GI,B,INC,'gi_transfer','ostd/src/sync/guard.rs:36','trait method; returns Self, no accessor'),
         (FR,B,INC,'gi_transfer','ostd/src/sync/guard.rs:36','&mut self final state unobservable')]
for i in range(134,140):
    put(i,GI,B,INC,'gi_guardian','ostd/src/sync/guard.rs:55','guard type has no accessor; PreemptDisabled/LocalIrqDisabled uninhabited external enums; assert-only concl')
put(140,VR,C,GAP,'vr_lock_new','ostd/src/sync/mutex.rs:53,202','Mutex::new pub const fn; MutexGuard::view pub open; lock has no ensures','partial:F-002')
put(141,OT,B,INC,'other_dbg','ostd/src/sync/mutex.rs:307','Formatter effect unmodeled')
put(142,GI,B,INC,'gi_rcu_read','ostd/src/sync/rcu/mod.rs:141','rcu module private and pub use commented out (sync/mod.rs:5,14); unreachable')
put(143,GI,B,INC,'gi_rcu_read','ostd/src/sync/rcu/mod.rs:768','same')
put(144,VR,C,GAP,'vr_rcuoption_none','ostd/src/sync/rcu/mod.rs:788,903','RcuOption exported cfg(ktest) (sync/mod.rs:25); Box<T>: NonNullPtr (non_null/mod.rs:239)','partial:F-017')
put(145,VR,C,GAP,'demand_rcuoption','ostd/src/sync/rcu/mod.rs:797','Some(Box::new(5)) constructible; get/is_none pub','partial:F-017')
put(146,GI,C,GAP,'demand_rcuoption','ostd/src/sync/rcu/mod.rs:833','RcuOptionReadGuard::get pub; guard acquisition; exact F-017 relation','exact:F-017')
put(147,IN,C,GAP,'demand_rcuoption','ostd/src/sync/rcu/mod.rs:903','is_none pub; Box recipe known','exact:F-017')
put(148,OT,B,INC,'other_dbg','ostd/src/sync/rcu/mod.rs:980','RcuDrop not exported (sync/mod.rs:14) and derive(Clone) only')
put(149,VR,B,INC,'other_blocked','ostd/src/sync/rcu/monitor.rs:45,103','RcuMonitor pub(super) in private module')
put(150,IN,B,INC,'other_blocked','ostd/src/sync/rcu/monitor.rs:136','private fn; type unreachable')
put(151,VR,B,INC,'other_blocked','ostd/src/sync/rcu/non_null/either.rs','private fn; rcu module private')
put(152,VR,C,GAP,'vr_guard','ostd/src/sync/rwarc.rs:70','RwArc::new ensures wf; RwLock guard view pub open','exact:F-006')
put(153,VR,C,GAP,'vr_guard','ostd/src/sync/rwarc.rs:75','same','exact:F-006')
put(154,VR,C,GAP,'vr_clone_ro','ostd/src/sync/rwarc.rs:111','clone_ro pub; RoArc::read returns view-able guard','partial:F-006')
put(155,VR,C,GAP,'vr_get_cloned','ostd/src/sync/rwarc.rs:174','get_cloned pub, T: Clone (u64); RwArc::new pub','exact:F-014')
put(156,VR,C,GAP,'vr_guard','ostd/src/sync/rwarc.rs:182','RoArc::read pub','exact:F-006')
put(157,VR,C,GAP,'vr_lock_new','ostd/src/sync/rwlock.rs:344,724','RwLock::new pub const fn; guard view pub open','partial:F-004')
put(158,VR,C,GAP,'vr_guard','ostd/src/sync/rwlock.rs:397,724','read pub; view open','partial:F-004')
put(159,VR,C,GAP,'vr_guard','ostd/src/sync/rwlock.rs:414,815','write pub; view open','partial:F-004')
put(160,VR,C,GAP,'vr_guard','ostd/src/sync/rwlock.rs:435,942','upread pub; view open','partial:F-004')
put(161,VR,C,GAP,'vr_guard','ostd/src/sync/rwlock.rs:449,724','try_read pub, verus_spec without ensures','exact:F-015')
put(162,VR,C,GAP,'vr_guard','ostd/src/sync/rwlock.rs:507,815','try_write pub','partial:F-004')
put(163,VR,C,GAP,'vr_guard','ostd/src/sync/rwlock.rs:563,942','try_upread pub','partial:F-004')
for i in (164,165,168):
    put(i,OT,B,INC,'other_dbg','ostd/src/sync/rwlock.rs','Formatter effect unmodeled')
put(166,VR,C,GAP,'vr_upgrade','ostd/src/sync/rwlock.rs:954','upgrade pub from upread guard; view open','partial:F-004')
put(167,VR,B,INC,'other_blocked','ostd/src/sync/rwlock.rs','try_upgrade private')
put(169,VR,C,GAP,'vr_lock_new','ostd/src/sync/rwmutex.rs:276,598','RwMutex::new pub const fn; guard view open','partial:F-005')
put(170,VR,C,GAP,'vr_guard','ostd/src/sync/rwmutex.rs:327,598','read pub','partial:F-005')
put(171,VR,C,GAP,'vr_guard','ostd/src/sync/rwmutex.rs:338,669','write pub','partial:F-005')
put(172,VR,C,GAP,'vr_guard','ostd/src/sync/rwmutex.rs:353,859','upread pub','partial:F-005')
put(173,VR,C,GAP,'vr_guard','ostd/src/sync/rwmutex.rs:361,598','try_read pub, verus_spec without ensures','exact:F-015')
put(174,VR,C,GAP,'vr_guard','ostd/src/sync/rwmutex.rs:415,669','try_write pub','partial:F-005')
put(175,VR,C,GAP,'vr_guard','ostd/src/sync/rwmutex.rs:464,859','try_upread pub','partial:F-005')
for i in (176,177,182):
    put(i,OT,B,INC,'other_dbg','ostd/src/sync/rwmutex.rs','Formatter effect unmodeled')
put(178,VR,C,GAP,'vr_upgrade','ostd/src/sync/rwmutex.rs:692','downgrade pub; view open','partial:F-005')
put(179,VR,B,INC,'other_blocked','ostd/src/sync/rwmutex.rs','try_downgrade private')
put(180,VR,C,GAP,'vr_upgrade','ostd/src/sync/rwmutex.rs:876','upgrade pub; view open','partial:F-005')
put(181,VR,B,INC,'other_blocked','ostd/src/sync/rwmutex.rs:906','try_upgrade private fn')
put(183,OT,B,INC,'other_dbg','ostd/src/sync/spin.rs:624','Formatter effect unmodeled')
put(184,VR,C,GAP,'let q = WaitQueue::new();\nassert(q.wake_all() == 0);','ostd/src/sync/wait.rs:97,189','new pub const fn; is_empty private, but wake_all pub returns 0 on a fresh queue')
put(185,VR,C,GAP,'let q = WaitQueue::new();\nassert(!q.wake_one());','ostd/src/sync/wait.rs:146','fresh queue has no waiters; wake_one pub')
put(186,VR,C,GAP,'vr_wait','ostd/src/sync/wait.rs:189','fresh queue; wake_all pub')
put(187,IN,B,INC,'other_blocked','ostd/src/sync/wait.rs:235','private fn')
put(188,VR,C,GAP,'let q = WaitQueue::default();\nassert(q.wake_all() == 0);','ostd/src/sync/wait.rs:268,189','Default delegates to new; observable via wake_all')
put(189,GI,N,INC,'let (w,wk) = Waiter::new_pair();\nassert(Arc::ptr_eq(w.task(), &wk.task_arc));','ostd/src/sync/wait.rs:328,412','Waiter::new_pair pub; Arc identity/Task accessor not expressible in spec')
put(190,OT,B,INC,'other_blocked','ostd/src/sync/wait.rs:449','external_body (unimplemented!)')
put(191,GI,B,INC,'gi_guardian','ostd/src/task/atomic_mode.rs:62','returns &dyn InAtomicMode; no identity/state accessor')
put(192,GI,B,INC,'gi_guardian','ostd/src/task/atomic_mode.rs:67','blanket impl; same')
put(193,GI,B,INC,'gi_guardian','ostd/src/task/atomic_mode.rs:73','dyn impl; same')
A2[194]=[(GI,B,INC,'gi_transfer','ostd/src/task/preempt/guard.rs:126','external_body; returns Self; no accessor'),
         (FR,B,INC,'gi_transfer','ostd/src/task/preempt/guard.rs:126','&mut self external_body')]
put(195,GI,B,INC,'gi_guardian','ostd/src/task/preempt/guard.rs:135','external_body; guard type no accessor')
put(196,OT,B,INC,'other_blocked','ostd/src/timer/jiffies.rs:9','global atomic static; value not spec-linked')
put(197,VR,N,GAP,'vr_jiffies','ostd/src/timer/jiffies.rs:69','Jiffies::new has ensures; elapsed has none; ELAPSED pub(crate) readable but atomic-load equality recipe')
put(198,GI,B,INC,'gi_guardian','ostd/src/trap/irq.rs:220','guard type no accessor; hardware state')
put(199,OT,B,INC,'other_blocked','ostd/src/trap/irq.rs:242','private external_body fn')
put(200,VR,C,GAP,'vr_either_clone','ostd/src/util/either.rs:11','pub enum derive(Clone,PartialEq); structural clone')
put(201,VR,C,GAP,'vr_either_side','ostd/src/util/either.rs:25','pub enum; no ensures')
put(202,VR,C,GAP,'vr_either_side','ostd/src/util/either.rs:35','pub enum; no ensures')
put(203,IN,C,GAP,'demand_either_side','ostd/src/util/either.rs:44','pub enum; no ensures')
put(204,IN,C,GAP,'demand_either_side','ostd/src/util/either.rs:52','pub enum; no ensures')

def slug(n):
    s=re.sub(r'[^a-z0-9]+','-',n.lower()).strip('-')
    return s[:80]


def make_draft(key,row,oname,meth):
    C=row['name'].split('::')[0].split('<')[0]
    if key=='vr_lock_new':
        if C=='Mutex': return "let l = Mutex::new(5);\nlet g = l.lock();\nassert(g.view() == 5);\nlet _held = g;"
        return f"let l = {C}::new(5);\nlet g = l.read();\nassert(g.view() == 5);\nlet _held = g;"
    if key=='vr_guard':
        if meth.startswith('try_'):
            return f"let l = {C}::new(5);\nif let Some(g) = l.{meth}() {{\n    assert(g.view() == 5);\n    let _held = g;\n}}"
        return f"let l = {C}::new(5);\nlet g = l.{meth}();\nassert(g.view() == 5);\nlet _held = g;"
    if key=='vr_upgrade':
        if meth=='upgrade':
            return f"let l = {C}::new(5);\nlet u = l.upread();\nlet g = u.upgrade();\nassert(g.view() == 5);\nlet _held = g;"
        if meth=='downgrade':
            return f"let l = {C}::new(5);\nlet g = l.write();\nlet d = g.downgrade();\nassert(d.view() == 5);\nlet _held = d;"
        return f"let l = {C}::new(5);\nlet u = l.upread();\nlet g = u.{meth}();\nassert(g.view() == 5);"
    if key=='vr_either_side':
        v='Left' if meth=='left' else 'Right'
        return f"let e = Either::<u64,u64>::{v}(5);\nassert(e.{meth}() == Some(5));"
    if key=='demand_either_side':
        v='Left' if meth=='is_left' else 'Right'
        return f"let e = Either::<u64,u64>::{v}(5);\nassert(e.{meth}());"
    if key=='fr_vmreader':
        if meth=='limit': return "let bytes=[1u8,2,3,4,5,6,7,8];\nlet mut r=VmReader::from(&bytes[..]);\nlet _=r.limit(4);\nassert(final(r).remain() <= 4);"
        return "let bytes=[1u8,2,3,4,5,6,7,8];\nlet mut r=VmReader::from(&bytes[..]);\nlet _=r.skip(4);\nassert(final(r).cursor.vaddr == old(r).cursor.vaddr + 4);"
    if key=='fr_vmwriter':
        if meth=='limit': return "let mut buf=[0u8;8];\nlet mut w=VmWriter::from(&mut buf[..]);\nlet _=w.limit(4);\nassert(final(w).remain_spec() <= 4);"
        return "let mut buf=[0u8;8];\nlet mut w=VmWriter::from(&mut buf[..]);\nlet _=w.skip(4);\nassert(final(w).cursor.vaddr == old(w).cursor.vaddr + 4);"
    if key=='fr_vmio_owner':
        return f"let mut v = <DmaStream<M> as VmIo<DmaStreamVmIoOwner<M>>>;\nlet _ = v.{meth}(.., Tracked(&mut owner));\nassert(final(owner).inv());  // ghost owner relation"
    if key=='fr_vmio_slice':
        return f"let mut v = <DmaStream<M> as VmIo<DmaStreamVmIoOwner<M>>>;\nlet _ = v.{meth}(.., &mut out, Tracked(&mut owner));\nassert(final(out).len() == old(out).len());"
    if key=='vr_dma_coherent':
        return f"let d = DmaCoherent::<M>::map(segment, true).unwrap();\nassert(d.{meth}() == expected);"
    if key=='vr_dma_stream':
        return f"let s: DmaStream<M> = ...;  // needs inner AtomicDataWithOwner recipe\nassert(s.{meth}() == expected);"
    if key=='vr_dma_slice':
        return f"let sl = DmaStreamSlice::new(stream, 0, 4);\nassert(sl.{meth}() == expected);"
    if key=='vr_dma_trait':
        return f"let x: impl HasDaddr = ...;\nassert(x.daddr() == expected);"
    if key=='vr_untyped_reader':
        return "let mem: &UntypedMem = ...;\nlet r = mem.reader();\nassert(r.cursor() == mem.as_virt_ptr());"
    if key=='vr_untyped_writer':
        return "let mem: &UntypedMem = ...;\nlet w = mem.writer();\nassert(w.cursor() == mem.as_virt_ptr());"
    if key=='vr_cursor_as_list':
        return "let (mut c, _) = list.cursor_front_mut();\nassert(c.as_list() == &list);"
    if key=='demand_link_is_untyped':
        return "let l = Link::<M>::new(meta);\nassert(l.is_untyped() == is_untyped_spec(meta));"
    if key=='fr_segment_repr':
        return "let (seg, Tracked(mut perm)) = Segment::from_unused(range, f);\nassert(final(perm) == old(perm));  // ghost repr_perm"
    if key=='vr_unique_from_shared':
        return "let u = UniqueFrame::try_from_shared(frame);\nassert(u is Ok);"
    if key=='fr_derefmut_self':
        return "let mut l = Link::<M>::new(meta);\nlet _ = l.deref_mut();\nassert(final(l) == old(l));  // private fields"
    if key=='vr_vmreader_cursor':
        return "let bytes=[1u8,2,3];\nlet r=VmReader::from(&bytes[..]);\nassert(r.cursor().vaddr == bytes.as_ptr() as usize);"
    if key=='demand_vmreader_has_remain':
        return "let bytes=[1u8,2,3];\nlet r=VmReader::from(&bytes[..]);\nassert(r.has_remain());"
    if key=='demand_vmwriter_has_avail':
        return "let mut buf=[0u8;3];\nlet w=VmWriter::from(&mut buf[..]);\nassert(w.has_avail());"
    if key=='vr_atomic_cpuset':
        return "let cs = CpuSet::new_empty();\nlet a = AtomicCpuSet::new(cs);\nassert(a.load(Ordering::Relaxed) == cs);"
    if key=='vr_linkedlist':
        return "let l = LinkedList::<M>::new();\nassert(l.is_empty());"
    if key=='vr_link':
        return "let l = Link::<M>::new(meta);\nassert(*(l.deref()) == meta);"
    if key=='vr_drhd':
        return "let d = Drhd::from_bytes(&hdr_bytes);  // requires valid header\nassert(d.register_base_addr() == expected);"
    if key=='vr_wait':
        return f"let q = WaitQueue::new();\nassert(q.{meth}() == 0);"
    if key=='vr_is_page_aligned':
        return "assert(is_page_aligned(4096));\nassert(!is_page_aligned(1));"
    if key=='vr_rangealloc':
        return "let r = RangeAllocator::new(0x1000..0x2000);\nassert(r == RangeAllocator);  // ZST, vacuous"
    if key=='vr_jiffies':
        return "let j = Jiffies::elapsed();\nassert(j.as_u64() == ELAPSED.load(Ordering::Relaxed));"
    if key=='gi_guardian':
        T=row['name'].split(' as ')[0].strip('<>') if ' as ' in row['name'] else 'PreemptDisabled'
        return f"let g = {T}::guard();\nassert(g.rel_guardian({T}));  // no such predicate in frozen face"
    if key=='gi_transfer':
        return "let mut g = disable_preempt();\nlet g2 = g.transfer_to();\nassert(g2.mode() == old(g).mode());  // no accessor"
    if key=='gi_rcu_read':
        return "let rcu = Rcu::<Box<u64>>::new(Box::new(5));\nlet g = rcu.read();\nassert(g.rel_rcu(rcu));"
    if key=='vr_framelloc_new':
        return "let o = FrameAllocOptions::new();\nassert(o.zeroed == true);  // field private, no accessor"
    if key=='fr_framealloc':
        return "let mut o = FrameAllocOptions::new();\no.zeroed(false);\nassert(final(o).zeroed == false);  // field private, no accessor"
    if key=='vr_rcuoption_none':
        return "let rcu = RcuOption::<Box<u64>>::new_none();\nlet g = rcu.read();\nassert(g.is_none());\nlet _held = g;"
    if key=='demand_rcuoption':
        return "let rcu = RcuOption::<Box<u64>>::new(Some(Box::new(5u64)));\nlet g = rcu.read();\nassert(!g.is_none());\nlet _held = g;"
    if key=='other_result':
        return f"let r = self.{meth}(..);\nassert(r is Ok);  // error/side-effect semantics not specified"
    if key=='other_dbg':
        return "let s = format!(\"{:?}\", x);\nassert(s == expected_string);  // fmt output unmodeled"
    if key=='other_ghost':
        return "// proof-only / ghost state; no exec obligation client"
    if key=='other_blocked':
        return "// no expressible/constructible obligation in frozen face"
    if key in DRAFT:
        d=DRAFT[key].replace('{C}',C).replace('{M}',meth).replace('{V}','0').replace('{EXPR}',f'self.{meth}(..)')
        return d
    return str(key)

def expand_draft(d,row,out,meth):
    if d in DRAFT: d=DRAFT[d]
    return (d.replace('{C}', row['name'].split('::')[0].replace('<','').replace('>',''))
             .replace('{M}', meth).replace('{V}', '0'))

out=[]
reg=Counter(); fam=Counter(); ctorc=Counter(); expc=Counter()
for row in rows:
    s=secs[row['sec']]; hr=row['hr']
    bullets=s['bullets']
    idx=[k for k in A if True]
    # find index of row in ordered rows
    ri=rows.index(row)+1
    per=None
    if ri in A2: per=A2[ri]
    else:
        fam0,ctor,exp,draft,ev,note,regv=A[ri]
        per=[(fam0,ctor,exp,draft,ev,note,regv)]*len(bullets)
    assert len(per)==len(bullets), (ri,row['name'],len(per),len(bullets))
    for bi,b in enumerate(bullets):
        kind='mut' if 'No `final(' in b else 'return'
        mm=re.search(r'`([^`]+)`',b)
        oname=mm.group(1) if mm else ('res' if kind=='return' else '?')
        tm=re.search(r'has type `([^`]+)`',b)
        otype=tm.group(1) if tm else '?'
        corrected=hr['ret'] or otype
        type_bug=('implementations' in s['heading'] and otype!=corrected)
        tup=per[bi]
        if len(tup)==6: tup=tup+('none',)
        f0,ctor,exp,draft,ev,note,regv=tup
        family=f0 if f0 else (FR if kind=='mut' else VR)
        meth=row['name'].split('::')[-1]
        draft_txt=make_draft(draft,row,oname,meth)
        rec=OrderedDict()
        rec['id']=f"C-{len(out)+1:03d}"
        rec['function']=row['name']
        rec['vir_path']=hr['vir']
        rec['artifact_location']=row['loc']
        rec['source_evidence']=ev
        rec['mode']=row['mode']
        rec['visibility']='private' if (hr['private'] or 'non-pub' in row['flags']) else 'pub'
        rec['artifact_flags']=row['flags']
        rec['artifact_section']='without-postcondition' if s['part'].startswith('Functions without') else 'with-postcondition'
        rec['post_source']=hr['post']
        rec['trait_decl']=hr['trait']
        rec['external_body']=hr['external_body']
        rec['frozen_symbol_found']=frozen_has(row)
        rec['artifact_inferred_name']=hr['inferred']
        rec['output_kind']=kind
        rec['output_name']=oname
        rec['output_type_artifact']=otype
        rec['output_type_head_results']=corrected
        rec['artifact_grouped_type_artifact_bug']=type_bug
        rec['invariant_only']=('Type invariant only' in b)
        rec['family']=family
        rec['obligation_draft']=draft_txt
        rec['constructibility']=ctor
        rec['constructibility_evidence']=ev
        rec['constructibility_note']=note
        if ctor==C: reason='public-ctor+observable'
        elif ctor==N: reason='needs-type-family-recipe'
        else:
            n=note.lower()
            if 'private' in n: reason='private-visibility'
            elif 'not exported' in n or 'unreachable' in n or 'not reachable' in n: reason='not-exported'
            elif 'external_body' in n: reason='external-body-no-spec'
            elif 'ghost' in n: reason='ghost-only'
            elif 'no accessor' in n or 'unobservable' in n or 'not observable' in n or 'no observable' in n or 'no spec observable' in n: reason='no-observable-state'
            elif 'derive' in n: reason='derived-no-spec-equality'
            else: reason='other-blocked'
        rec['constructibility_reason']=reason
        rec['prereg_expected']=exp
        rec['prereg_status']='hypothesis-before-run (unverified)'
        rec['dedup_key']=f"{ {'value-readback':'vr','guard-identity':'gi','is_none-demand':'demand','final-relation':'fr','other':'other'}[family] }:{slug(row['name'])}:{bi+1}"
        REG_O={'VmSpace::default':'partial:F-012','VmSpace::new':'partial:F-012',
                'VmSpace::cursor':'partial:F-012','VmSpace::cursor_mut':'partial:F-012',
                "CursorMut<'a>::protect_next":'partial:F-012',
                'RangeAllocator::new':'partial:F-009'}
        if row['name'] in REG_O: regv=REG_O[row['name']]
        rec['registry_overlap']=regv
        out.append(rec)
        fam[family]+=1; ctorc[ctor]+=1; expc[exp]+=1
        if rec['registry_overlap']!='none': reg[rec['registry_overlap'].split(':')[0]]+=1


# ---- 43da079 census cross-check (peer M7a artifact) ----
CEN_PATH='/tmp/m7a-census-43da079/census.json'
try:
    CEN=json.load(open(CEN_PATH))['functions']
except Exception:
    CEN=[]
from collections import defaultdict as _dd
_cby=_dd(list)
for _c in CEN: _cby[_c['source']['file']].append(_c)
def _pick_census(rec):
    if not CEN: return None
    f=rec['artifact_location'].rsplit(':',1)[0]; meth=rec['function'].split('::')[-1]
    try: ln=int(rec['artifact_location'].rsplit(':',1)[1])
    except Exception: ln=0
    cs=[c for c in _cby.get(f,[]) if c['name']==meth]
    if not cs: return None
    if len(cs)==1: return cs[0]
    toks=re.findall(r'[A-Za-z_][A-Za-z0-9_]*', rec['output_type_head_results'] or '')
    if toks:
        last=toks[-1]
        c2=[c for c in cs if any(last in (o['type'] or '') for o in c['outputs'])]
        if len(c2)==1: return c2[0]
    tr=rec.get('trait_decl')
    if tr:
        short=tr.split('::')[-1]
        c2=[c for c in cs if c['trait'] and c['trait']['method'].split('::')[-1]==short]
        if len(c2)==1: return c2[0]
    cs.sort(key=lambda c:abs(c['source']['line']-ln))
    return cs[0]
for rec in out:
    c=_pick_census(rec)
    if c is None:
        rec['frozen_census_status']='absent-in-43da079'
        rec['frozen_census_path']=None
        f=rec['artifact_location'].rsplit(':',1)[0]
        if '/apic/' in f: rec['revision_note']='APIC module absent from 43da079 VIR/census'
        elif 'fmt' in rec['function']: rec['revision_note']='Debug fmt impl absent/commented in 43da079 census'
        elif 'as_atomic_mode_guard' in rec['function']: rec['revision_note']='AsAtomicModeGuard impl absent from 43da079 VIR/census'
        else: rec['revision_note']='no 43da079 counterpart'
    else:
        rec['frozen_census_status']='reproduced-in-43da079'
        rec['frozen_census_path']=c['path']
        rec['revision_note']='function present in 43da079 census with at least one finding'
json.dump(out, open('/tmp/m7c-candidates/candidates.json','w'), indent=1)
print('records',len(out))
print('family',dict(fam))
print('constructibility',dict(ctorc))
print('expected',dict(expc))
print('registry overlap',dict(reg))
from collections import Counter as C2
print('exact keys',C2(r['registry_overlap'] for r in out if r['registry_overlap'].startswith('exact')))
print('partial keys',C2(r['registry_overlap'] for r in out if r['registry_overlap'].startswith('partial')))
