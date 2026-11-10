package event

import (
	"testing"
	"time"

	v1 "k8s.io/api/core/v1"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	"k8s.io/client-go/tools/record"
	testclocks "k8s.io/utils/clock/testing"
)

// recordToSink PATCHes with a nil patch only when an event with count above 1 has none. A
// recorder's event starts at count 1, so count passes 1 only through eventObserve, in the
// branch that also builds the patch.
func TestRepeatAlwaysHasPatch(t *testing.T) {
	c := record.NewEventCorrelatorWithOptions(record.CorrelatorOptions{Clock: testclocks.NewFakeClock(time.Now())})
	for i := 1; i <= 3; i++ {
		now := metav1.Now()
		r, err := c.EventCorrelate(&v1.Event{
			ObjectMeta:     metav1.ObjectMeta{Name: "x.1", Namespace: "default", Annotations: map[string]string{"a": "b"}},
			InvolvedObject: v1.ObjectReference{Kind: "MyType", Namespace: "default", Name: "x", UID: "u", APIVersion: "sample.example.com/v1alpha1", ResourceVersion: "5"},
			Reason:         "CannotConnectToProvider", Message: "same", FirstTimestamp: now, LastTimestamp: now, Count: 1, Type: "Warning",
			Source: v1.EventSource{Component: "managed/mytype"},
		})
		t.Logf("count=%d patch=%s err=%v", r.Event.Count, r.Patch, err)
		if int(r.Event.Count) != i || (i > 1 && (r.Patch == nil || err != nil)) {
			t.Fatalf("observation %d: count %d, patch %q, err %v", i, r.Event.Count, r.Patch, err)
		}
	}
}
