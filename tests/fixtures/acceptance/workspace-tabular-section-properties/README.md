# Tabular-section properties workspace

Isolated Designer XML configuration for issue #1689. The `src` tree was exported
by 1C 8.3.27.2074 after importing and applying a separate seed configuration.
It contains hierarchical `Catalog.Items`, nonhierarchical `Catalog.FlatItems`,
`ChartOfCharacteristicTypes.Properties`, `Document.Order`,
`BusinessProcess.Process`, `ExchangePlan.Exchange`, `Report.Report`, and
`DataProcessor.Processor`. Each owns `Lines` with `Quantity` and `Text` columns.
`Task.Tasks` is the required business-process dependency.

The initial section properties are `Use=ForItem` where supported and
`FillChecking=DontCheck`. The fixture is a full configuration suitable for
platform import; the existing catalog-property fixture is deliberately separate.
