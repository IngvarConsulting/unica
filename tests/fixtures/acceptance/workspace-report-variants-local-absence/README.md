# Local report variants-storage absence

Synthetic read-only fixture, not a native platform dump. The ordinary Report has no local VariantsStorage scalar. The adopted extension Report also omits it, while its registered parent refers to SettingsStorage.Storage. Reading either local descriptor must not synthesize an inherited value or an unreadable relation.
