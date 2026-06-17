package config

import (
	"os"
	"path/filepath"
	"strings"
	"testing"
)

// validSecret is a 32-byte secret we use in tests that need a non-placeholder value.
const validSecret = "this-is-a-32-byte-test-secret-x!"

// minimalConfigYAML is a small but valid config.yaml fixture. Tests override
// the jwt.secret as needed.
const minimalConfigYAML = `
jwt:
  secret: "` + validSecret + `"
  ttl_hours: 24
  issuer: edgecloud
`

func writeConfig(t *testing.T, body string) string {
	t.Helper()
	dir := t.TempDir()
	path := filepath.Join(dir, "config.yaml")
	if err := os.WriteFile(path, []byte(body), 0o600); err != nil {
		t.Fatalf("write config: %v", err)
	}
	return path
}

func TestLoad_RejectsPlaceholderSecrets(t *testing.T) {
	// Snapshot the map so we exercise every entry. The map is package-level
	// and must not be mutated by tests.
	for placeholder := range insecureJWTSecretValues {
		t.Run("placeholder="+placeholder, func(t *testing.T) {
			// Clear any JWT_SECRET from the surrounding environment so the
			// YAML value is what Load sees.
			t.Setenv("JWT_SECRET", "")

			body := "jwt:\n  secret: \"" + placeholder + "\"\n"
			path := writeConfig(t, body)

			_, err := Load(path)
			if err == nil {
				t.Fatalf("expected error for placeholder secret %q, got nil", placeholder)
			}
			if !strings.Contains(err.Error(), "placeholder") {
				t.Errorf("error %q should mention 'placeholder'", err.Error())
			}
		})
	}
}

func TestLoad_RejectsShortSecret(t *testing.T) {
	t.Setenv("JWT_SECRET", "")

	// 31 ASCII bytes — one short of the minimum.
	short := strings.Repeat("a", 31)
	body := "jwt:\n  secret: \"" + short + "\"\n"
	path := writeConfig(t, body)

	_, err := Load(path)
	if err == nil {
		t.Fatal("expected error for 31-byte secret, got nil")
	}
	if !strings.Contains(err.Error(), "32 bytes") {
		t.Errorf("error %q should mention the 32-byte minimum", err.Error())
	}
}

func TestLoad_AcceptsValidSecret(t *testing.T) {
	t.Setenv("JWT_SECRET", "")

	path := writeConfig(t, minimalConfigYAML)
	cfg, err := Load(path)
	if err != nil {
		t.Fatalf("Load: %v", err)
	}
	if cfg.JWT.Secret != validSecret {
		t.Errorf("JWT.Secret = %q, want %q", cfg.JWT.Secret, validSecret)
	}
}

func TestLoad_EnvVarOverridesYAML(t *testing.T) {
	// YAML contains a different (but also valid) secret; the env var must win.
	yamlSecret := strings.Repeat("y", 32)
	envSecret := strings.Repeat("e", 32)

	body := "jwt:\n  secret: \"" + yamlSecret + "\"\n"
	path := writeConfig(t, body)

	t.Setenv("JWT_SECRET", envSecret)
	cfg, err := Load(path)
	if err != nil {
		t.Fatalf("Load: %v", err)
	}
	if cfg.JWT.Secret != envSecret {
		t.Errorf("JWT.Secret = %q, want env value %q", cfg.JWT.Secret, envSecret)
	}
}
