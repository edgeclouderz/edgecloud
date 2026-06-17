package service

import (
	"crypto/sha256"
	"encoding/hex"
)

// sha256HexOf returns the lowercase hex-encoded SHA-256 of the input. Used as
// the stable lookup_hash for API keys (migration 006) and as a cheap pre-filter
// in authentication paths.
func sha256HexOf(s string) string {
	h := sha256.Sum256([]byte(s))
	return hex.EncodeToString(h[:])
}
