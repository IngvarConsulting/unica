//! Cross-component integration of the RLM owner and trusted source policies.
#[cfg(test)]
mod tests {
    #[test]
    fn rlm_coordination_paths_separate_source_roots_under_the_pair_root() {
        unica_adapter_rlm::index::rlm_coordination_contract(|identity, context, name, root| {
            let actor =
                crate::infrastructure::source_workspace::new_actor(identity, context.clone())
                    .unwrap();
            let binding = actor.bind_provider_root(name, root).unwrap();
            actor.provider_source_lease(&binding).unwrap()
        });
    }
}
