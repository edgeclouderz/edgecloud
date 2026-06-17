package service

import (
	"context"
	"errors"
	"sync"
	"testing"
	"time"
)

// -----------------------------------------------------------------------
// Mock repo — exercises the GC service without a live DB.
// -----------------------------------------------------------------------

type mockLogGCRepo struct {
	mu    sync.Mutex
	calls []time.Time // cutoff timestamps passed to each DeleteOlderThan call
	err   error
	delay time.Duration // optional sleep before returning (simulates slow DB)
}

func (m *mockLogGCRepo) DeleteOlderThan(ctx context.Context, cutoff time.Time) (int64, error) {
	if m.delay > 0 {
		select {
		case <-time.After(m.delay):
		case <-ctx.Done():
			return 0, ctx.Err()
		}
	}
	m.mu.Lock()
	defer m.mu.Unlock()
	m.calls = append(m.calls, cutoff)
	if m.err != nil {
		return 0, m.err
	}
	return 0, nil
}

func (m *mockLogGCRepo) callCount() int {
	m.mu.Lock()
	defer m.mu.Unlock()
	return len(m.calls)
}

func (m *mockLogGCRepo) lastCutoff() (time.Time, bool) {
	m.mu.Lock()
	defer m.mu.Unlock()
	if len(m.calls) == 0 {
		return time.Time{}, false
	}
	return m.calls[len(m.calls)-1], true
}

// -----------------------------------------------------------------------
// Tests
// -----------------------------------------------------------------------

// TestLogGC_DeletesOldRows: Run fires immediately, then once per interval.
// We use a long interval so only the immediate sweep happens in the test
// window, and we cancel the context before the first tick would fire.
func TestLogGC_DeletesOldRows(t *testing.T) {
	repo := &mockLogGCRepo{}
	svc := NewLogGCService(repo)

	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()

	const (
		interval  = 10 * time.Second // far longer than the test duration
		retention = 7 * 24 * time.Hour
	)

	done := make(chan struct{})
	go func() {
		svc.Run(ctx, interval, retention)
		close(done)
	}()

	// The Run loop's immediate sweep runs synchronously before returning
	// to the select, so we don't strictly need to wait — but a small
	// yield makes the assertion deterministic on busy CI.
	time.Sleep(20 * time.Millisecond)

	cutoff, ok := repo.lastCutoff()
	if !ok {
		t.Fatal("DeleteOlderThan was not called on first sweep")
	}
	if got := repo.callCount(); got != 1 {
		t.Errorf("DeleteOlderThan call count = %d, want 1 (interval hasn't elapsed yet)", got)
	}

	// Cutoff should be roughly now - retention.
	wantCutoff := time.Now().Add(-retention)
	delta := cutoff.Sub(wantCutoff)
	if delta < -2*time.Second || delta > 2*time.Second {
		t.Errorf("cutoff = %s, want ~%s (delta %s)", cutoff, wantCutoff, delta)
	}

	cancel()
	select {
	case <-done:
	case <-time.After(2 * time.Second):
		t.Fatal("Run did not exit after ctx cancellation")
	}
}

// TestLogGC_RetentionFromConfig: different retention values produce
// different cutoffs. Verifies the retention parameter is plumbed through.
func TestLogGC_RetentionFromConfig(t *testing.T) {
	repo := &mockLogGCRepo{}
	svc := NewLogGCService(repo)

	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()

	const interval = 10 * time.Second

	// First run with 1-hour retention.
	go svc.Run(ctx, interval, 1*time.Hour)
	time.Sleep(20 * time.Millisecond)
	cutoff1, _ := repo.lastCutoff()
	cancel()
	time.Sleep(20 * time.Millisecond) // give Run a moment to exit

	// Second run with 7-day retention.
	ctx2, cancel2 := context.WithCancel(context.Background())
	defer cancel2()
	go svc.Run(ctx2, interval, 7*24*time.Hour)
	time.Sleep(20 * time.Millisecond)
	cutoff2, _ := repo.lastCutoff()
	cancel2()

	// cutoff2 should be ~7 days earlier than cutoff1.
	delta := cutoff1.Sub(cutoff2)
	wantDelta := 7*24*time.Hour - 1*time.Hour
	if delta < wantDelta-time.Second || delta > wantDelta+time.Second {
		t.Errorf("cutoff delta = %s, want ~%s", delta, wantDelta)
	}
}

// TestLogGC_TickerFiresAtInterval: with a short interval, Run should call
// DeleteOlderThan multiple times within a small window. Validates that the
// ticker path is actually wired (not just the immediate sweep).
func TestLogGC_TickerFiresAtInterval(t *testing.T) {
	repo := &mockLogGCRepo{}
	svc := NewLogGCService(repo)

	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()

	const (
		interval  = 30 * time.Millisecond
		retention = 1 * time.Hour
	)

	done := make(chan struct{})
	go func() {
		svc.Run(ctx, interval, retention)
		close(done)
	}()

	// Wait long enough for several ticks. The first sweep is immediate;
	// each subsequent sweep is every 30ms. Over 150ms we expect 4-6 calls.
	time.Sleep(150 * time.Millisecond)
	cancel()
	<-done

	got := repo.callCount()
	if got < 3 {
		t.Errorf("DeleteOlderThan call count = %d, want at least 3 (immediate + 2+ ticks)", got)
	}
}

// TestLogGC_RepoErrorDoesNotStopLoop: a transient DB error is logged and the
// loop continues. The next tick should still attempt the delete.
func TestLogGC_RepoErrorDoesNotStopLoop(t *testing.T) {
	repo := &mockLogGCRepo{err: errors.New("simulated DB outage")}
	svc := NewLogGCService(repo)

	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()

	const (
		interval  = 30 * time.Millisecond
		retention = 1 * time.Hour
	)

	done := make(chan struct{})
	go func() {
		svc.Run(ctx, interval, retention)
		close(done)
	}()

	time.Sleep(100 * time.Millisecond)
	cancel()
	<-done

	// Multiple attempts must have happened despite the error.
	if got := repo.callCount(); got < 2 {
		t.Errorf("DeleteOlderThan call count = %d, want >= 2 (loop must continue after errors)", got)
	}
}
