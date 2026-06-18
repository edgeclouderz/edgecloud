package service

import (
	"context"
	"log"
	"time"
)

// logEntryRepoForGC is the subset of *repository.LogEntryRepository used by
// LogGCService. Defining it locally keeps tests mockable without a live DB.
type logEntryRepoForGC interface {
	DeleteOlderThan(ctx context.Context, cutoff time.Time) (int64, error)
}

// LogGCService periodically deletes log rows older than the configured
// retention. It runs as a background goroutine for the lifetime of the
// control-plane process and exits cleanly when ctx is cancelled.
//
// Errors are logged and the loop continues — a transient DB failure should
// not silently halt the GC forever, but the operator should also see it.
type LogGCService struct {
	repo logEntryRepoForGC
}

func NewLogGCService(repo logEntryRepoForGC) *LogGCService {
	return &LogGCService{repo: repo}
}

// Run blocks until ctx is cancelled. The first sweep fires immediately
// (operationally useful — when the process restarts we don't want to wait
// `interval` before deleting old rows); subsequent sweeps tick at `interval`.
//
// interval and retention are passed in by main.go so they can be tuned via
// env vars (LOG_GC_INTERVAL, LOG_RETENTION) without changing this struct.
//
// If either duration is non-positive the service refuses to run: interval<=0
// would busy-loop (ticker fires immediately on every iteration) and
// retention<=0 would compute a future cutoff and wipe the entire logs table.
// The operator sees a clear log line and the GC stays disabled until the
// env vars are fixed and the process restarted.
func (s *LogGCService) Run(ctx context.Context, interval, retention time.Duration) {
	if interval <= 0 || retention <= 0 {
		log.Printf("log_gc: invalid interval=%s retention=%s; refusing to run", interval, retention)
		return
	}

	// runOnce is a closure so the immediate-first-sweep path and the
	// ticker path use the same delete-and-log logic.
	runOnce := func() {
		// Skip the DELETE roundtrip if we're already shutting down. The
		// repository itself short-circuits on a cancelled ctx, but
		// checking here avoids a wasted pool acquire + log on the
		// shutdown path and keeps the immediate-first-sweep from
		// issuing a DELETE we're about to cancel.
		if ctx.Err() != nil {
			return
		}
		cutoff := time.Now().Add(-retention)
		deleted, err := s.repo.DeleteOlderThan(ctx, cutoff)
		if err != nil {
			if ctx.Err() != nil {
				return // shutting down — expected
			}
			log.Printf("log_gc: delete failed (cutoff=%s): %v", cutoff.Format(time.RFC3339), err)
			return
		}
		if deleted > 0 {
			log.Printf("log_gc: deleted %d rows older than %s", deleted, cutoff.Format(time.RFC3339))
		}
	}

	runOnce()

	ticker := time.NewTicker(interval)
	defer ticker.Stop()
	for {
		select {
		case <-ctx.Done():
			return
		case <-ticker.C:
			runOnce()
		}
	}
}
