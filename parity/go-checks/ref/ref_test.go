package ref

import (
	"context"
	"encoding/json"
	"io"
	"net/http"
	"net/http/httptest"
	"testing"

	"github.com/crossplane/crossplane-runtime/v2/pkg/reconciler/managed"
	"github.com/example/provider-baseline/apis/sample/v1alpha1"
	"k8s.io/apimachinery/pkg/api/meta"
	metav1 "k8s.io/apimachinery/pkg/apis/meta/v1"
	"k8s.io/apimachinery/pkg/runtime"
	"k8s.io/client-go/rest"
	"sigs.k8s.io/controller-runtime/pkg/client"
)

type RefMyType struct{ v1alpha1.MyType }

func (r *RefMyType) DeepCopyObject() runtime.Object { return &RefMyType{MyType: *r.MyType.DeepCopy()} }
func (r *RefMyType) ResolveReferences(_ context.Context, _ client.Reader) error {
	r.Spec.ForProvider.ConfigurableField = "resolved"
	return nil
}

type req struct{ method, ctype, body string }

func setup(t *testing.T) (client.Client, *[]req) {
	gvk := v1alpha1.MyTypeGroupVersionKind
	s := runtime.NewScheme()
	s.AddKnownTypeWithName(gvk, &RefMyType{})
	metav1.AddToGroupVersion(s, gvk.GroupVersion())
	m := meta.NewDefaultRESTMapper(nil)
	m.Add(gvk, meta.RESTScopeNamespace)
	reqs := &[]req{}
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		b, _ := io.ReadAll(r.Body)
		*reqs = append(*reqs, req{r.Method + " " + r.URL.String(), r.Header.Get("Content-Type"), string(b)})
		live := map[string]any{"apiVersion": gvk.GroupVersion().String(), "kind": gvk.Kind,
			"metadata": map[string]any{"name": "x", "namespace": "default", "resourceVersion": "2",
				"annotations": map[string]any{"crossplane.io/external-name": "x"}},
			"spec": map[string]any{"forProvider": map[string]any{"configurableField": "orig"}}}
		w.Header().Set("Content-Type", "application/json")
		_ = json.NewEncoder(w).Encode(live)
	}))
	t.Cleanup(srv.Close)
	c, err := client.New(&rest.Config{Host: srv.URL}, client.Options{Scheme: s, Mapper: m})
	if err != nil {
		t.Fatal(err)
	}
	return c, reqs
}

func fresh() *RefMyType {
	o := &RefMyType{}
	o.Name, o.Namespace, o.ResourceVersion = "x", "default", "1"
	o.Spec.ForProvider.ConfigurableField = "orig"
	return o
}

// As after the cached Get, which sets the GVK (cache_reader.go:104).
func TestAfterCachedGet(t *testing.T) {
	c, reqs := setup(t)
	o := fresh()
	o.SetGroupVersionKind(v1alpha1.MyTypeGroupVersionKind)
	if err := managed.NewAPISimpleReferenceResolver(c).ResolveReferences(context.Background(), o); err != nil {
		t.Fatal(err)
	}
	assertApplyBody(t, *reqs)
}

// As after NameAsExternalName's Update in the same reconcile.
func TestAfterTypedUpdate(t *testing.T) {
	c, reqs := setup(t)
	o := fresh()
	o.SetGroupVersionKind(v1alpha1.MyTypeGroupVersionKind)
	if err := c.Update(context.Background(), o); err != nil {
		t.Fatal(err)
	}
	t.Logf("GVK after Update: %q", o.GetObjectKind().GroupVersionKind().String())
	if err := managed.NewAPISimpleReferenceResolver(c).ResolveReferences(context.Background(), o); err != nil {
		t.Fatal(err)
	}
	assertApplyBody(t, *reqs)
}

const wantApply = `{"apiVersion":"sample.example.com/v1alpha1","kind":"MyType","spec":{"forProvider":{"configurableField":"resolved"}}}`

func assertApplyBody(t *testing.T, reqs []req) {
	t.Helper()
	last := reqs[len(reqs)-1]
	t.Logf("%s [%s] %s", last.method, last.ctype, last.body)
	if last.ctype != "application/apply-patch+yaml" || last.body != wantApply {
		t.Fatalf("apply body %s, want %s", last.body, wantApply)
	}
}
