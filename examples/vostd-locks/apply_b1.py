#!/usr/bin/env python3
"""B1: Mutex::try_lock always returns None, keeping the current contract."""
import sys
p=sys.argv[1]+"/ostd/src/sync/mutex.rs"
s=open(p).read()
start=s.index("    pub fn try_lock(&self) -> Option<MutexGuard<'_, T>> {")
end=s.index("\n    }\n", start)+len("\n    }\n")
s=s[:start]+"    pub fn try_lock(&self) -> Option<MutexGuard<'_, T>> {\n        None\n    }\n"+s[end:]
open(p,'w').write(s)
print("B1 applied")
