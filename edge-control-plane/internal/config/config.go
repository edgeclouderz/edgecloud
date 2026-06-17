package config

import (
	"fmt"
	"os"
	"strconv"

	"gopkg.in/yaml.v3"
)

// Config holds all application configuration.
type Config struct {
	Database  DatabaseConfig  `yaml:"database"`
	NATS      NATSConfig      `yaml:"nats"`
	App       AppConfig       `yaml:"app"`
	Storage   StorageConfig   `yaml:"storage"`
	JWT       JWTConfig       `yaml:"jwt"`
	Migration MigrationConfig `yaml:"migration"`
}

type DatabaseConfig struct {
	Host     string `yaml:"host"`
	Port     int    `yaml:"port"`
	User     string `yaml:"user"`
	Password string `yaml:"password"`
	Name     string `yaml:"name"`
	SSLMode  string `yaml:"sslmode"`
}

type NATSConfig struct {
	URL string `yaml:"url"`
}

type AppConfig struct {
	Host string `yaml:"host"`
	Port int    `yaml:"port"`
	Env  string `yaml:"env"`
}

type StorageConfig struct {
	ArtifactPath string `yaml:"artifact_path"`
}

type JWTConfig struct {
	Secret string `yaml:"secret"`
	TTL    int    `yaml:"ttl_hours"`
	Issuer string `yaml:"issuer"`
}

// DSN returns the PostgreSQL connection string.
func (d *DatabaseConfig) DSN() string {
	return fmt.Sprintf(
		"host=%s port=%d user=%s password=%s dbname=%s sslmode=%s",
		d.Host, d.Port, d.User, d.Password, d.Name, d.SSLMode,
	)
}

// Load reads config from a YAML file, then overrides with environment variables.
func Load(path string) (*Config, error) {
	data, err := os.ReadFile(path)
	if err != nil {
		return nil, fmt.Errorf("reading config file: %w", err)
	}

	var cfg Config
	if err := yaml.Unmarshal(data, &cfg); err != nil {
		return nil, fmt.Errorf("parsing config: %w", err)
	}

	// Override with env vars if set
	if v := os.Getenv("DATABASE_HOST"); v != "" {
		cfg.Database.Host = v
	}
	if v := os.Getenv("DATABASE_PORT"); v != "" {
		port, err := strconv.Atoi(v)
		if err != nil {
			return nil, fmt.Errorf("DATABASE_PORT must be a valid integer: %w", err)
		}
		cfg.Database.Port = port
	}
	if v := os.Getenv("DATABASE_USER"); v != "" {
		cfg.Database.User = v
	}
	if v := os.Getenv("DATABASE_PASSWORD"); v != "" {
		cfg.Database.Password = v
	}
	if v := os.Getenv("DATABASE_NAME"); v != "" {
		cfg.Database.Name = v
	}
	if v := os.Getenv("DATABASE_SSLMODE"); v != "" {
		cfg.Database.SSLMode = v
	}
	if v := os.Getenv("NATS_URL"); v != "" {
		cfg.NATS.URL = v
	}
	if v := os.Getenv("APP_HOST"); v != "" {
		cfg.App.Host = v
	}
	if v := os.Getenv("APP_PORT"); v != "" {
		port, err := strconv.Atoi(v)
		if err != nil {
			return nil, fmt.Errorf("APP_PORT must be a valid integer: %w", err)
		}
		cfg.App.Port = port
	}
	if v := os.Getenv("APP_ENV"); v != "" {
		cfg.App.Env = v
	}
	if v := os.Getenv("STORAGE_ARTIFACT_PATH"); v != "" {
		cfg.Storage.ArtifactPath = v
	}
	if v := os.Getenv("JWT_SECRET"); v != "" {
		cfg.JWT.Secret = v
	}
	if v := os.Getenv("JWT_TTL_HOURS"); v != "" {
		ttl, err := strconv.Atoi(v)
		if err != nil {
			return nil, fmt.Errorf("JWT_TTL_HOURS must be a valid integer: %w", err)
		}
		cfg.JWT.TTL = ttl
	}
	if v := os.Getenv("JWT_ISSUER"); v != "" {
		cfg.JWT.Issuer = v
	}

	// Override with migration config env vars
	if v := os.Getenv("EDGE_MIGRATE_PATH"); v != "" {
		cfg.Migration.EdgeMigratePath = v
	}
	if v := os.Getenv("WASI_SDK_PATH"); v != "" {
		cfg.Migration.WasiSdkPath = v
	}

	// Defaults for JWT config
	if cfg.JWT.Issuer == "" {
		cfg.JWT.Issuer = "edgecloud"
	}
	if cfg.JWT.TTL == 0 {
		cfg.JWT.TTL = 24
	}

	// Reject insecure JWT secrets. Operators frequently ship with the
	// default `change-me-in-production` placeholder and forget to override
	// it; failing startup is louder and safer than silently running with a
	// publicly-known secret. (Audit finding #2 — also referenced by tests.)
	if err := validateJWTSecret(cfg.JWT.Secret); err != nil {
		return nil, err
	}

	return &cfg, nil
}

// insecureJWTSecretValues is the set of well-known placeholder JWT secrets
// that must not be accepted in production. Operators must override these
// with a real secret via `JWT_SECRET` env var or `jwt.secret` config field.
//
// The set is small and curated — adding entries requires a code review so
// a typo doesn't accidentally invalidate a legitimate operator secret.
var insecureJWTSecretValues = map[string]struct{}{
	"change-me-in-production": {},
	"changeme":                {},
	"secret":                  {},
	"":                        {},
	"default":                 {},
	"insecure":                {},
}

// validateJWTSecret enforces two rules: (1) the secret must not match a
// known placeholder, and (2) it must be at least 32 bytes long. Both checks
// are needed because a placeholder secret can be arbitrarily long.
func validateJWTSecret(s string) error {
	if _, ok := insecureJWTSecretValues[s]; ok {
		return fmt.Errorf("jwt.secret %q is a known placeholder; set JWT_SECRET or jwt.secret to a unique value", s)
	}
	if len(s) < 32 {
		return fmt.Errorf("jwt.secret must be at least 32 bytes (got %d)", len(s))
	}
	return nil
}

// MigrationConfig holds paths to migration toolchain binaries.
type MigrationConfig struct {
	EdgeMigratePath string `yaml:"edge_migrate_path" env:"EDGE_MIGRATE_PATH" envDefault:"edge-migrate"`
	WasiSdkPath     string `yaml:"wasi_sdk_path"     env:"WASI_SDK_PATH"     envDefault:"/usr/local/wasi-sdk/bin"`
}
