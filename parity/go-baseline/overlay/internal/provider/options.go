package provider

import (
	"github.com/alecthomas/kingpin/v2"

	"github.com/crossplane/crossplane-runtime/v2/pkg/controller"
)

// Endpoint is the base URL of the parity fake external service.
var Endpoint = new(string)

func Flags(app *kingpin.Application) {
	Endpoint = app.Flag("external-endpoint", "Base URL of the external service.").
		Envar("EXTERNAL_ENDPOINT").
		Default("http://fake-external.parity-external.svc.cluster.local:8080").
		String()
}

func Configure(_ *controller.Options) error {
	return nil
}
