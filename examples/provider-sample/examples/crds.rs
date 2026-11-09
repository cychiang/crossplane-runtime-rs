//! Prints the provider's CRDs as a JSON stream, for `hack/crd-diff` to compare with the Go
//! baseline's `package/crds`.

fn main() {
    for crd in provider_sample::apis::crds() {
        println!("{}", serde_json::to_string_pretty(&crd).expect("serializable CRD"));
    }
}
