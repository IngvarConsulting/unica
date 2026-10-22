# Catalog tabular attribute ToolTip

Read-only fixture for the six observations in #1559. The Catalog descriptor
is adapted from the retained platform-export fixture used by
`catalog-attribute-properties`, independently of Unica's metadata writer.

The `ПараметрыПрописи` tooltip retains the reported Russian text and adds
an English translation with an XML entity to exercise lossless localization.
`Damaged.xml` contains a deliberately malformed tooltip; it is an adversarial
input, not a claimed valid platform export. Other boundary variants are built
explicitly by the Rust tests. This fixture does not claim a new platform
import/export round trip.
