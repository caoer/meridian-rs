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

	// Node spans per wire-contract v1 laws (block spans exclude final newline):
	// frontmatter: bytes 0..19  ("---\ntitle: demo\n---")
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
	fmSpan := [2]int{0, 19}
	aStart := idx("# Alpha")
	bStart := idx("## Beta")
	aSpan := [2]int{aStart, aStart + len("# Alpha")}
	bSpan := [2]int{bStart, bStart + len("## Beta")}
	fmt.Printf("frontmatter span %v => %q\n", fmSpan, string(fb[fmSpan[0]:fmSpan[1]]))
	fmt.Printf("heading Alpha span %v => %q\n", aSpan, string(fb[aSpan[0]:aSpan[1]]))
	fmt.Printf("heading Beta  span %v => %q\n", bSpan, string(fb[bSpan[0]:bSpan[1]]))

	// SECTION spans (resolve): heading line through end of subtree (wire §6.1)
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
}
