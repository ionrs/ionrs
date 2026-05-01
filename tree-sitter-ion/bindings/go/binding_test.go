package tree_sitter_ion_test

import (
	"testing"

	tree_sitter "github.com/smacker/go-tree-sitter"
	"github.com/tree-sitter/tree-sitter-ion"
)

func TestCanLoadGrammar(t *testing.T) {
	language := tree_sitter.NewLanguage(tree_sitter_ion.Language())
	if language == nil {
		t.Errorf("Error loading Ion grammar")
	}
}
