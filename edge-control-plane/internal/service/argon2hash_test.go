package service

import (
	"strings"
	"testing"
)

func TestHashAPIKey_Format(t *testing.T) {
	encoded, err := HashAPIKey("test-raw-key")
	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}
	// PHC format: $argon2id$v=19$m=65536,t=1,p=4$<salt>$<key>
	if !strings.HasPrefix(encoded, "$argon2id$v=19$m=") {
		t.Errorf("encoded hash does not start with argon2id PHC header: %q", encoded)
	}
	parts := strings.Split(encoded, "$")
	if len(parts) != 6 {
		t.Errorf("expected 6 parts, got %d: %v", len(parts), parts)
	}
}

func TestHashAPIKey_UniqueSalts(t *testing.T) {
	// Two hashes of the same input must differ (random salt).
	a, _ := HashAPIKey("same-input")
	b, _ := HashAPIKey("same-input")
	if a == b {
		t.Error("two hashes of the same input are identical — salt is not random")
	}
}

func TestHashAPIKey_EmptyInput(t *testing.T) {
	if _, err := HashAPIKey(""); err == nil {
		t.Error("expected error for empty input, got nil")
	}
}

func TestVerifyAPIKey_RoundTrip(t *testing.T) {
	raw := "this-is-a-secret-api-key-12345"
	encoded, err := HashAPIKey(raw)
	if err != nil {
		t.Fatalf("hash failed: %v", err)
	}
	ok, err := VerifyAPIKey(raw, encoded)
	if err != nil {
		t.Fatalf("verify failed: %v", err)
	}
	if !ok {
		t.Error("VerifyAPIKey returned false for the original key")
	}
}

func TestVerifyAPIKey_WrongKey(t *testing.T) {
	encoded, _ := HashAPIKey("the-real-key")
	ok, err := VerifyAPIKey("a-different-key", encoded)
	if err != nil {
		t.Fatalf("verify returned error: %v", err)
	}
	if ok {
		t.Error("VerifyAPIKey returned true for a wrong key")
	}
}

func TestVerifyAPIKey_MalformedEncoded(t *testing.T) {
	cases := []string{
		"",
		"not-a-phc-string",
		"$argon2id$v=19$m=65536,t=1,p=4$short$short",
		"$bcrypt$v=19$m=65536,t=1,p=4$AAAA$BBBB",
		"$$$$$$",
	}
	for _, c := range cases {
		t.Run(c, func(t *testing.T) {
			_, err := VerifyAPIKey("any-key", c)
			if err == nil {
				t.Errorf("expected error for malformed input %q, got nil", c)
			}
		})
	}
}

// TestVerifyAPIKey_RejectsMalformedParams covers the hand-rolled PHC
// parser: the old fmt.Sscanf code silently accepted trailing junk and
// partial matches. Verify the new parser refuses each malformed segment
// loudly.
func TestVerifyAPIKey_RejectsMalformedParams(t *testing.T) {
	cases := []struct {
		name string
		enc  string
	}{
		{"bad_version_segment", "$argon2id$ver=19$m=65536,t=1,p=4$AAAA$BBBB"},
		{"version_non_numeric", "$argon2id$v=abc$m=65536,t=1,p=4$AAAA$BBBB"},
		{"version_with_junk", "$argon2id$v=19junk$m=65536,t=1,p=4$AAAA$BBBB"},
		{"missing_m", "$argon2id$v=19$t=1,p=4$AAAA$BBBB"},
		{"m_non_numeric", "$argon2id$v=19$m=abc,t=1,p=4$AAAA$BBBB"},
		{"m_negative", "$argon2id$v=19$m=-1,t=1,p=4$AAAA$BBBB"},
		{"t_zero", "$argon2id$v=19$m=65536,t=0,p=4$AAAA$BBBB"},
		{"m_zero", "$argon2id$v=19$m=0,t=1,p=4$AAAA$BBBB"},
		{"p_zero", "$argon2id$v=19$m=65536,t=1,p=0$AAAA$BBBB"},
		{"unknown_param", "$argon2id$v=19$m=65536,t=1,p=4,q=9$AAAA$BBBB"},
		{"missing_equals", "$argon2id$v=19$m65536,t1,p4$AAAA$BBBB"},
		{"empty_segment", "$argon2id$v=19$$AAAA$BBBB"},
	}
	for _, c := range cases {
		t.Run(c.name, func(t *testing.T) {
			_, err := VerifyAPIKey("any-key", c.enc)
			if err == nil {
				t.Errorf("expected error for %q, got nil", c.enc)
			}
		})
	}
}

// TestVerifyAPIKey_ZeroParamsRejected is the boundary case for the
// parameter check: argon2.IDKey itself panics on zero parameters in
// some versions, so verify VerifyAPIKey rejects them before that path.
func TestVerifyAPIKey_ZeroParamsRejected(t *testing.T) {
	cases := []string{
		"$argon2id$v=19$m=0,t=1,p=4$AAAA$BBBB",
		"$argon2id$v=19$m=65536,t=0,p=4$AAAA$BBBB",
		"$argon2id$v=19$m=65536,t=1,p=0$AAAA$BBBB",
	}
	for _, c := range cases {
		t.Run(c, func(t *testing.T) {
			ok, err := VerifyAPIKey("any-key", c)
			if err == nil {
				t.Errorf("expected error for zero-param %q, got nil (ok=%v)", c, ok)
			}
		})
	}
}
