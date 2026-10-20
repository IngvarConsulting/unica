use std::any::TypeId;
use unica_analysis_services::provider_host::ProviderHostKey;
use unica_artifact_delivery::shared::DeliveryWorkKey;
use unica_execution::runtime_jobs::RuntimeWorkKey;
use unica_workspace::actor::IndexWorkIdentity;

// A conversion would let a foreign owner's key cross a typed admission seam.
// In that case both marker implementations match and type inference fails.
macro_rules! assert_no_conversion {
    ($from:ty => $to:ty) => {
        const _: fn() = || {
            trait AmbiguousIfConvertible<Marker> {
                fn check() {}
            }
            struct Converted;
            impl<T: ?Sized> AmbiguousIfConvertible<()> for T {}
            impl<T: ?Sized + Into<$to>> AmbiguousIfConvertible<Converted> for T {}
            let _ = <$from as AmbiguousIfConvertible<_>>::check;
        };
    };
}

assert_no_conversion!(ProviderHostKey => DeliveryWorkKey);
assert_no_conversion!(IndexWorkIdentity => DeliveryWorkKey);
assert_no_conversion!(RuntimeWorkKey => DeliveryWorkKey);
assert_no_conversion!(DeliveryWorkKey => ProviderHostKey);
assert_no_conversion!(IndexWorkIdentity => ProviderHostKey);
assert_no_conversion!(RuntimeWorkKey => ProviderHostKey);
assert_no_conversion!(DeliveryWorkKey => IndexWorkIdentity);
assert_no_conversion!(ProviderHostKey => IndexWorkIdentity);
assert_no_conversion!(RuntimeWorkKey => IndexWorkIdentity);
assert_no_conversion!(DeliveryWorkKey => RuntimeWorkKey);
assert_no_conversion!(ProviderHostKey => RuntimeWorkKey);
assert_no_conversion!(IndexWorkIdentity => RuntimeWorkKey);

#[test]
fn exact_key_vocabulary_covers_delivery_index_provider_and_runtime() {
    let delivery = TypeId::of::<DeliveryWorkKey>();
    let index = TypeId::of::<IndexWorkIdentity>();
    let provider = TypeId::of::<ProviderHostKey>();
    let runtime = TypeId::of::<RuntimeWorkKey>();
    assert_ne!(delivery, index);
    assert_ne!(index, provider);
    assert_ne!(provider, runtime);
    assert_ne!(delivery, provider);
    assert_ne!(delivery, runtime);
    assert_ne!(index, runtime);
}
