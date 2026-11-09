package mytype

import (
	"context"

	"github.com/crossplane/crossplane-runtime/v2/pkg/controller"
	"github.com/crossplane/crossplane-runtime/v2/pkg/errors"
	"github.com/crossplane/crossplane-runtime/v2/pkg/meta"
	"github.com/crossplane/crossplane-runtime/v2/pkg/reconciler/managed"
	"github.com/crossplane/crossplane-runtime/v2/pkg/resource"
	xpv2 "github.com/crossplane/crossplane/apis/v2/core/v2"
	ctrl "sigs.k8s.io/controller-runtime"

	"github.com/example/provider-baseline/apis/sample/v1alpha1"
	"github.com/example/provider-baseline/internal/provider"
)

const errNotMyType = "managed resource is not a MyType custom resource"

// External maps a MyType onto the thing named by its external name in the
// parity fake service. The runtime's default initializer sets the external
// name to metadata.name before the first Observe.
type External struct {
	client *provider.Client
}

func NewExternal(c *provider.Client) *External {
	return &External{client: c}
}

func ReconcilerOptions(_ ctrl.Manager, _ controller.Options) ([]managed.ReconcilerOption, error) {
	return nil, nil
}

// Observe deliberately has no early return for a resource being deleted: the
// reconciler only calls Delete when Observe reports the thing still exists.
func (e *External) Observe(ctx context.Context, mg resource.Managed) (managed.ExternalObservation, error) {
	cr, ok := mg.(*v1alpha1.MyType)
	if !ok {
		return managed.ExternalObservation{}, errors.New(errNotMyType)
	}
	t, err := e.client.Get(ctx, meta.GetExternalName(cr))
	if err != nil {
		return managed.ExternalObservation{}, err
	}
	if t == nil {
		return managed.ExternalObservation{ResourceExists: false}, nil
	}
	cr.Status.AtProvider.ConfigurableField = t.ConfigurableField
	cr.Status.SetConditions(xpv2.Available())
	return managed.ExternalObservation{
		ResourceExists:   true,
		ResourceUpToDate: t.ConfigurableField == cr.Spec.ForProvider.ConfigurableField,
	}, nil
}

func (e *External) Create(ctx context.Context, mg resource.Managed) (managed.ExternalCreation, error) {
	cr, ok := mg.(*v1alpha1.MyType)
	if !ok {
		return managed.ExternalCreation{}, errors.New(errNotMyType)
	}
	cr.Status.SetConditions(xpv2.Creating())
	t, err := e.client.Put(ctx, meta.GetExternalName(cr), provider.Thing{ConfigurableField: cr.Spec.ForProvider.ConfigurableField})
	if err != nil {
		return managed.ExternalCreation{}, err
	}
	cr.Status.AtProvider.ConfigurableField = t.ConfigurableField
	return managed.ExternalCreation{}, nil
}

func (e *External) Update(ctx context.Context, mg resource.Managed) (managed.ExternalUpdate, error) {
	cr, ok := mg.(*v1alpha1.MyType)
	if !ok {
		return managed.ExternalUpdate{}, errors.New(errNotMyType)
	}
	t, err := e.client.Put(ctx, meta.GetExternalName(cr), provider.Thing{ConfigurableField: cr.Spec.ForProvider.ConfigurableField})
	if err != nil {
		return managed.ExternalUpdate{}, err
	}
	cr.Status.AtProvider.ConfigurableField = t.ConfigurableField
	return managed.ExternalUpdate{}, nil
}

func (e *External) Delete(ctx context.Context, mg resource.Managed) (managed.ExternalDelete, error) {
	cr, ok := mg.(*v1alpha1.MyType)
	if !ok {
		return managed.ExternalDelete{}, errors.New(errNotMyType)
	}
	cr.Status.SetConditions(xpv2.Deleting())
	return managed.ExternalDelete{}, e.client.Delete(ctx, meta.GetExternalName(cr))
}

func (e *External) Disconnect(_ context.Context) error {
	return nil
}
