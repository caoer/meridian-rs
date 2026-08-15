package main

// Worked example generator for node-rev-merkle-spec: builds the spec's example
// workspace, prints every hash pre-image and hash (blake3-32B, node_rev truncation).

import (
	"encoding/binary"
	"fmt"

	"lukechampine.com/blake3"
)

func b3(data []byte) [32]byte { return blake3.Sum256(data) }

func hex8(h [32]byte) string  { return fmt.Sprintf("%x", h[:8]) }
func hexFull(h [32]byte) string { return fmt.Sprintf("%x", h[:]) }

func main() {
	// The example file: tasks/x.md
	file := "---\ntitle: demo\n---\n\n# Alpha\n\nbody line one\n\n## Beta\n\nbeta body\n"
	fb := []byte(file)
	fmt.Printf("file bytes (%d):\n%q\n\n", len(fb), file)

	// Node spans per the standing wire laws (wire-contract.md §1 span
	// sub-laws; frontmatter is terminator-INCLUSIVE — fence-to-fence
	// container, span-lawed with the section family, §18 row 3):
	// frontmatter: bytes 0..20  ("---\ntitle: demo\n---\n")
	// heading Alpha: find "# Alpha"
	// heading Beta: "## Beta"
	idx := func(s string) int {
		for i := 0; i+len(s) <= len(fb); i++ {
			if string(fb[i:i+len(s)]) == s {
				return i
			}
		}
		return -1
	}
	fmSpan := [2]int{0, 20}
	aStart := idx("# Alpha")
	bStart := idx("## Beta")
	aSpan := [2]int{aStart, aStart + len("# Alpha")}
	bSpan := [2]int{bStart, bStart + len("## Beta")}
	fmt.Printf("frontmatter span %v => %q\n", fmSpan, string(fb[fmSpan[0]:fmSpan[1]]))
	fmt.Printf("heading Alpha span %v => %q\n", aSpan, string(fb[aSpan[0]:aSpan[1]]))
	fmt.Printf("heading Beta  span %v => %q\n", bSpan, string(fb[bSpan[0]:bSpan[1]]))

	// SECTION spans (resolve): heading line through end of subtree
	// (wire-contract.md §1 span sub-laws)
	// Alpha section: from aStart to EOF; Beta section: from bStart to EOF.
	alphaSec := fb[aStart:]
	betaSec := fb[bStart:]
	fmt.Printf("\nsection Alpha (heading-inclusive) span [%d,%d)\n", aStart, len(fb))
	fmt.Printf("section Beta  (heading-inclusive) span [%d,%d)\n", bStart, len(fb))

	// node_rev = first 16 hex chars (8 bytes) of blake3 over the node's span bytes
	fmRev := b3(fb[fmSpan[0]:fmSpan[1]])
	aRev := b3(alphaSec)
	bRev := b3(betaSec)
	fmt.Printf("\nnode_rev(frontmatter) = blake3(%q)[:8] = %s\n", string(fb[fmSpan[0]:fmSpan[1]]), hex8(fmRev))
	fmt.Printf("node_rev(#Alpha section) = %s\n", hex8(aRev))
	fmt.Printf("node_rev(#Alpha/Beta section) = %s\n", hex8(bRev))

	// FILE leaf hash = blake3 over the raw file bytes (full 32B)
	xLeaf := b3(fb)
	fmt.Printf("\nleaf(tasks/x.md) = blake3(raw bytes) = %s\n", hexFull(xLeaf))

	// Second file: notes.md
	notes := "# Notes\n\nhello\n"
	nLeaf := b3([]byte(notes))
	fmt.Printf("leaf(notes.md) %q = %s\n", notes, hexFull(nLeaf))

	// Interior node: dir "tasks" containing x.md
	enc := func(entries []struct {
		name string
		typ  byte
		hash [32]byte
	}) []byte {
		var out []byte
		var buf [10]byte
		for _, e := range entries {
			k := binary.PutUvarint(buf[:], uint64(len(e.name)))
			out = append(out, buf[:k]...)
			out = append(out, e.name...)
			out = append(out, e.typ)
			out = append(out, e.hash[:]...)
		}
		return out
	}
	type entry = struct {
		name string
		typ  byte
		hash [32]byte
	}
	tasksPre := enc([]entry{{"x.md", 0, xLeaf}})
	tasksHash := b3(tasksPre)
	fmt.Printf("\ninterior(tasks/) pre-image = varint(4) 'x.md' 0x00 leaf  (%d bytes): %x\n", len(tasksPre), tasksPre)
	fmt.Printf("interior(tasks/) = %s\n", hexFull(tasksHash))

	// Workspace root: entries sorted by name: notes.md (file), tasks (dir)
	rootPre := enc([]entry{{"notes.md", 0, nLeaf}, {"tasks", 1, tasksHash}})
	rootHash := b3(rootPre)
	fmt.Printf("\nroot pre-image (%d bytes): %x\n", len(rootPre), rootPre)
	fmt.Printf("root = b3:%s\n", hexFull(rootHash))

	// Incremental update: splice replaces Beta section body "beta body\n" -> "beta body v2\n"
	newFile := "---\ntitle: demo\n---\n\n# Alpha\n\nbody line one\n\n## Beta\n\nbeta body v2\n"
	nfb := []byte(newFile)
	xLeaf2 := b3(nfb)
	tasksPre2 := enc([]entry{{"x.md", 0, xLeaf2}})
	tasksHash2 := b3(tasksPre2)
	rootPre2 := enc([]entry{{"notes.md", 0, nLeaf}, {"tasks", 1, tasksHash2}})
	rootHash2 := b3(rootPre2)
	bStart2 := 0
	for i := 0; i+7 <= len(nfb); i++ {
		if string(nfb[i:i+7]) == "## Beta" {
			bStart2 = i
		}
	}
	bRev2 := b3(nfb[bStart2:])
	fmt.Printf("\nAFTER splice of Beta section:\n")
	fmt.Printf("node_rev(Beta) %s -> %s\n", hex8(bRev), hex8(bRev2))
	fmt.Printf("leaf(tasks/x.md) -> %s\n", hexFull(xLeaf2))
	fmt.Printf("interior(tasks/) -> %s\n", hexFull(tasksHash2))
	fmt.Printf("root -> b3:%s\n", hexFull(rootHash2))
	// unchanged: notes.md leaf
	fmt.Printf("unchanged: leaf(notes.md) still %s\n", hex8(nLeaf))

	// ——— Merkle law 2 (spec §4.2, worked in §5.1): the same workspace under
	// the fixed-256 radix child map. Pre-images follow §4.2.2 exactly:
	// "mrk2.vtx" ‖ varint(len(ext)) ‖ ext ‖ terminal_frame ‖ children_frame,
	// then the §4.2.3 wrap "mrk2.dir" ‖ vhash. These values are pinned in
	// §5.1 and are the radix card's byte-identity gate.
	uleb := func(n int) []byte {
		var buf [10]byte
		k := binary.PutUvarint(buf[:], uint64(n))
		return buf[:k]
	}
	vtx := func(ext, terminal, children []byte) [32]byte {
		pre := []byte("mrk2.vtx")
		pre = append(pre, uleb(len(ext))...)
		pre = append(pre, ext...)
		pre = append(pre, terminal...)
		pre = append(pre, children...)
		return b3(pre)
	}
	oneTerminal := func(kind byte, h [32]byte) []byte {
		return append([]byte{0x01, kind}, h[:]...)
	}
	noTerminal := []byte{0x00}
	noChildren := []byte{0x00} // children_frame with n = 0
	dirWrap := func(vh [32]byte) [32]byte {
		return b3(append([]byte("mrk2.dir"), vh[:]...))
	}
	twoChildren := func(s1 byte, v1 [32]byte, s2 byte, v2 [32]byte) []byte {
		out := []byte{0x02, s1}
		out = append(out, v1[:]...)
		out = append(out, s2)
		out = append(out, v2[:]...)
		return out
	}

	// tasks/: one vertex — ext "x.md", file terminal, no children.
	vX := vtx([]byte("x.md"), oneTerminal(0x00, xLeaf), noChildren)
	dirTasks := dirWrap(vX)
	// root map: v_n (ext "otes.md"), v_t (ext "asks"), root fans 0x6e/0x74.
	vN := vtx([]byte("otes.md"), oneTerminal(0x00, nLeaf), noChildren)
	vT := vtx([]byte("asks"), oneTerminal(0x01, dirTasks), noChildren)
	vRoot := vtx([]byte{}, noTerminal, twoChildren(0x6e, vN, 0x74, vT))
	fp2 := dirWrap(vRoot)
	fmt.Printf("\n=== merkle law 2 (spec §5.1) ===\n")
	fmt.Printf("vhash(v_x, tasks/ map) = %s\n", hexFull(vX))
	fmt.Printf("dir(tasks/)            = %s\n", hexFull(dirTasks))
	fmt.Printf("vhash(v_n)             = %s\n", hexFull(vN))
	fmt.Printf("vhash(v_t)             = %s\n", hexFull(vT))
	fmt.Printf("vhash(root)            = %s\n", hexFull(vRoot))
	fmt.Printf("fingerprint (law 2)    = %s\n", hexFull(fp2))

	// The same §5 splice under law 2: only the x.md key path recomputes.
	vX2 := vtx([]byte("x.md"), oneTerminal(0x00, xLeaf2), noChildren)
	dirTasks2 := dirWrap(vX2)
	vT2 := vtx([]byte("asks"), oneTerminal(0x01, dirTasks2), noChildren)
	vRoot2 := vtx([]byte{}, noTerminal, twoChildren(0x6e, vN, 0x74, vT2))
	fp22 := dirWrap(vRoot2)
	fmt.Printf("AFTER splice: dir(tasks/)  -> %s\n", hexFull(dirTasks2))
	fmt.Printf("AFTER splice: fingerprint  -> %s\n", hexFull(fp22))
	fmt.Printf("unchanged: vhash(v_n) still %s\n", hex8(vN))
}
