// NDEF-area parsing tests — the same vectors as the Node.js and Rust
// ports, mirroring the Python node's behavior (Type 5 capability
// container + TLV stream, Text/URI records, hex-dump fallback).
package main

import (
	"bytes"
	"strings"
	"testing"
)

// cc is a Type 5 capability container (magic 0xE1).
var cc = []byte{0xE1, 0x40, 0x40, 0x00}

func area(tlvs ...[]byte) []byte {
	data := append([]byte{}, cc...)
	for _, t := range tlvs {
		data = append(data, t...)
	}
	return data
}

// textHello is an NDEF short record: well-known Text, "en", "Hello".
var textHello = []byte{0xD1, 0x01, 0x08, 0x54, 0x02, 0x65, 0x6E, 0x48, 0x65, 0x6C, 0x6C, 0x6F}

func TestParseNdefArea(t *testing.T) {
	cases := []struct {
		name string
		data []byte
		want Content
	}{
		{"blank zeros", make([]byte, scanLength), Content{Kind: "empty"}},
		{"blank ff", bytes.Repeat([]byte{0xFF}, scanLength), Content{Kind: "empty"}},
		{"empty scan", nil, Content{Kind: "empty"}},
		{"no capability container", []byte{0xDE, 0xAD, 0xBE, 0xEF}, Content{Kind: "data", Value: "DEADBEEF"}},
		{"short garbage", []byte{0x12}, Content{Kind: "data", Value: "12"}},
		{
			"text record",
			area([]byte{0x03, 0x0C}, textHello, []byte{0xFE}),
			Content{Kind: "text", Value: "Hello"},
		},
		{
			"uri record",
			area([]byte{0x03, 0x0D, 0xD1, 0x01, 0x09, 0x55, 0x04}, []byte("seeed.cc"), []byte{0xFE}),
			Content{Kind: "uri", Value: "https://seeed.cc"},
		},
		{
			"utf16 text record",
			area([]byte{0x03, 0x0B, 0xD1, 0x01, 0x07, 0x54, 0x82, 0x65, 0x6E, 0x48, 0x00, 0x69, 0x00}, []byte{0xFE}),
			Content{Kind: "text", Value: "Hi"},
		},
		{
			"three-byte tlv length",
			area([]byte{0x03, 0xFF, 0x00, 0x0C}, textHello, []byte{0xFE}),
			Content{Kind: "text", Value: "Hello"},
		},
		{
			"padding and unknown tlv skipped",
			area([]byte{0x00, 0x01, 0x02, 0xAA, 0xBB, 0x03, 0x0C}, textHello, []byte{0xFE}),
			Content{Kind: "text", Value: "Hello"},
		},
		{
			"unknown record type",
			area([]byte{0x03, 0x06, 0xD2, 0x01, 0x02, 0x78, 0xDE, 0xAD, 0xFE}),
			Content{Kind: "data", Value: "DEAD"},
		},
		{"terminator only", area([]byte{0xFE}), Content{Kind: "empty"}},
		{"zero-length ndef tlv", area([]byte{0x03, 0x00, 0xFE}), Content{Kind: "empty"}},
		{
			"message truncated by scan window",
			area([]byte{0x03, 0x10, 0xD1, 0x01}),
			Content{Kind: "data", Value: "D101"},
		},
		{
			"text record without payload",
			area([]byte{0x03, 0x04, 0xD1, 0x01, 0x00, 0x54, 0xFE}),
			Content{Kind: "data", Value: "D1010054"},
		},
	}

	for _, c := range cases {
		if got := parseNdefArea(c.data); got != c.want {
			t.Errorf("%s: got %+v, want %+v", c.name, got, c.want)
		}
	}
}

func TestDataHexDumpIsCapped(t *testing.T) {
	got := parseNdefArea(bytes.Repeat([]byte{0xAB}, 80))
	want := Content{Kind: "data", Value: strings.Repeat("AB", dataHexCap)}
	if got != want {
		t.Errorf("got %d hex chars, want %d", len(got.Value), len(want.Value))
	}
}
