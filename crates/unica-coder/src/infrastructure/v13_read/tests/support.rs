use crate::application::result_store::ViewCursorStore;
use crate::application::v13::find::{FindRequest, FindResult};
use crate::application::v13::view::{ViewReadAuthority, ViewService};
use crate::domain::address::QualifiedAddress;
use crate::domain::cancellation::CancellationToken;
use crate::domain::code_intelligence::ProviderDeadline;
use crate::domain::platform_profile::PlatformProfile;
use crate::domain::project_sources::SourceSetKind;
use crate::domain::workspace::WorkspaceContext;
use crate::infrastructure::platform::filesystem::RetainedDirectoryCapability;
use crate::infrastructure::v13_read::LogicalViewReadAuthority;
use crate::infrastructure::v13_read_port::ProviderReadAuthority;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// Коды отказов из ответа. Сообщение провала несёт только их: разбор в панике
/// затягивает в текст всю нагрузку вместе с идентификаторами фикстуры, и
/// сканер справедливо не отличает такой текст от записи в журнал.
pub(super) fn refusal_codes(result: &crate::domain::invocation::DomainResult) -> Vec<&str> {
    result
        .diagnostics
        .iter()
        .filter_map(|diagnostic| diagnostic.get("code").and_then(serde_json::Value::as_str))
        .collect()
}

pub(super) struct RealExternalReaderFixture {
    _root: tempfile::TempDir,
    pub(super) context: WorkspaceContext,
    pub(super) processor: PathBuf,
    pub(super) report: PathBuf,
    pub(super) cancellation: CancellationToken,
}

impl RealExternalReaderFixture {
    pub(super) fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let cache = root.path().join("cache");
        let processor = root.path().join("processor");
        let report = root.path().join("report");
        fs::create_dir_all(&cache).unwrap();
        fs::create_dir_all(&processor).unwrap();
        fs::create_dir_all(&report).unwrap();
        write_external_artifact(&processor, "ExternalDataProcessor", "Import");
        write_external_artifact(&processor, "ExternalDataProcessor", "Импорт");
        write_external_artifact(&report, "ExternalReport", "Sales");
        write_external_artifact(&report, "ExternalReport", "Продажи");
        fs::copy(
            fixture_path("platform_8_3_27/staged_dump_roots/ConfigDumpInfo.xml"),
            processor.join("ConfigDumpInfo.xml"),
        )
        .unwrap();
        fs::copy(
            fixture_path("platform_8_3_27/staged_dump_roots/ConfigDumpInfo.xml"),
            report.join("ConfigDumpInfo.xml"),
        )
        .unwrap();
        fs::write(
            root.path().join("v8project.yaml"),
            "format: DESIGNER\nsource-set:\n  - name: artifact_processor\n    type: EXTERNAL_PROCESSOR\n    path: processor\n  - name: artifact_report\n    type: EXTERNAL_REPORT\n    path: report\n",
        )
        .unwrap();
        let canonical_root = fs::canonicalize(root.path()).unwrap();
        let processor = fs::canonicalize(processor).unwrap();
        let report = fs::canonicalize(report).unwrap();
        let context = WorkspaceContext {
            cwd: canonical_root.clone(),
            workspace_root: canonical_root,
            cache_root: cache,
            workspace_epoch: 1,
        };
        Self {
            _root: root,
            context,
            processor,
            report,
            cancellation: CancellationToken::new(),
        }
    }

    pub(super) fn read_authority(
        &self,
        source_set: &str,
        kind: SourceSetKind,
    ) -> LogicalViewReadAuthority<'_> {
        let source = if kind == SourceSetKind::ExternalProcessor {
            &self.processor
        } else {
            &self.report
        };
        let root = Arc::new(RetainedDirectoryCapability::open(source).unwrap());
        LogicalViewReadAuthority::new(
            &self.cancellation,
            source_set,
            format!("actor-{source_set}"),
            kind,
            root,
            PlatformProfile::v8_3_27(),
        )
    }

    pub(super) fn operation_read_authority(
        &self,
        source_set: &str,
        kind: SourceSetKind,
    ) -> LogicalViewReadAuthority<'_> {
        let source = if kind == SourceSetKind::ExternalProcessor {
            &self.processor
        } else {
            &self.report
        };
        let root = Arc::new(RetainedDirectoryCapability::open(source).unwrap());
        let deadline = ProviderDeadline::no_deadline();
        LogicalViewReadAuthority::with_read_authority(
            &self.cancellation,
            ProviderReadAuthority::new(source_set, format!("actor-{source_set}"), kind, root),
            PlatformProfile::v8_3_27(),
            deadline,
        )
    }
}

pub(super) fn write_external_artifact(source: &Path, kind: &str, name: &str) {
    let (form_name, command_name) = match name {
        "Импорт" => ("Основная", "Выполнить"),
        "Продажи" => ("Основная", "Сформировать"),
        _ => ("Main", "Run"),
    };
    write(
        &source.join(format!("{name}.xml")),
        &format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20">
  <{kind} uuid="10000000-0000-4000-8000-000000000010">
    <Properties><Name>{name}</Name></Properties>
    <ChildObjects><Form>{form_name}</Form><Command uuid="10000000-0000-4000-8000-000000000012"><Properties><Name>{command_name}</Name></Properties></Command></ChildObjects>
  </{kind}>
</MetaDataObject>"#,
        ),
    );
    write(
        &source.join(format!("{name}/Forms/{form_name}.xml")),
        &format!(
            r#"<?xml version="1.0" encoding="UTF-8"?><MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Form uuid="10000000-0000-4000-8000-000000000011"><Properties><Name>{form_name}</Name><FormType>Managed</FormType></Properties></Form></MetaDataObject>"#
        ),
    );
    write(
        &source.join(format!("{name}/Forms/{form_name}/Ext/Form.xml")),
        r#"<?xml version="1.0" encoding="UTF-8"?><Form xmlns="http://v8.1c.ru/8.3/xcf/logform" version="2.20"><ChildItems/></Form>"#,
    );
    write(
        &source.join(format!("{name}/Commands/{command_name}/Ext/CommandModule.bsl")),
        "&AtClient\nProcedure CommandProcessing(CommandParameter, CommandExecuteParameters)\nEndProcedure\n",
    );
    write(
        &source.join(format!("{name}/Ext/ObjectModule.bsl")),
        "Procedure Execute()\nEndProcedure\n",
    );
}

/// Proves the typed reader reaches an address. `find` no longer traverses the
/// logical tree, so reader coverage is asserted against the reader itself.
pub(super) fn assert_reader_reaches(authority: &LogicalViewReadAuthority<'_>, addresses: &[&str]) {
    for at in addresses {
        let address = QualifiedAddress::parse(at).unwrap_or_else(|error| panic!("{at}: {error}"));
        let admitted = authority
            .snapshot(&address)
            .unwrap_or_else(|error| panic!("{at}: {error:?}"));
        authority
            .read_exact(&address, &admitted)
            .unwrap_or_else(|error| panic!("{at}: {error:?}"));
    }
}

/// `find` is a layout directory now, so reader-coverage tests probe the typed
/// reader directly. The shim keeps the assertion shape those tests already use.
pub(super) struct ReaderReach<'a> {
    authorities: Vec<(&'a str, &'a LogicalViewReadAuthority<'a>)>,
}

impl<'a> ReaderReach<'a> {
    pub(super) fn new(authorities: Vec<(&'a str, &'a LogicalViewReadAuthority<'a>)>) -> Self {
        Self { authorities }
    }

    pub(super) fn find(&self, request: FindRequest) -> FindResult {
        let at = request.query();
        let reached = QualifiedAddress::parse(at).is_ok_and(|address| {
            self.authorities
                .iter()
                .filter(|(name, _)| *name == address.source_set())
                .any(|(_, authority)| {
                    authority.snapshot(&address).is_ok_and(|admitted| {
                        match authority.read_exact(&address, &admitted) {
                            Ok(_) => true,
                            // WebSocketClient is the one profile identity whose
                            // source layout 8.3.27 leaves unspecified: it
                            // answers a typed `provider_unavailable` while the
                            // reader still knows the address. No other provider
                            // failure counts as coverage.
                            Err(error) => {
                                error.code().as_str() == "provider_unavailable"
                                    && address.segments().first().is_some_and(|segment| {
                                        segment.kind()
                                            == crate::domain::address::NodeKind::WebSocketClient
                                    })
                            }
                        }
                    })
                })
        });
        FindResult::reader_probe(at, reached)
    }
}

pub(super) fn write_metadata_owner(
    source: &Path,
    directory: &str,
    kind: &str,
    name: &str,
    child_objects: &str,
) {
    write(
        &source.join(format!("{directory}/{name}.xml")),
        &format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20">
  <{kind} uuid="10000000-0000-4000-8000-000000000020">
    <Properties><Name>{name}</Name></Properties>
    <ChildObjects>{child_objects}</ChildObjects>
  </{kind}>
</MetaDataObject>"#,
        ),
    );
}

pub(super) struct RealReaderFixture {
    pub(super) root: tempfile::TempDir,
    pub(super) context: WorkspaceContext,
    pub(super) source: PathBuf,
    pub(super) cancellation: CancellationToken,
}

impl RealReaderFixture {
    pub(super) fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("src");
        let cache = root.path().join("cache");
        fs::create_dir_all(&source).unwrap();
        fs::create_dir_all(&cache).unwrap();
        fs::write(
                root.path().join("v8project.yaml"),
                "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
            )
            .unwrap();
        let config = fixture_text("xdto/enterprise-data-minimal/Configuration.xml");
        fs::write(
                source.join("Configuration.xml"),
                replace_child_objects(
                    &config,
                    "\n\t\t\t<Constant>MainCurrency</Constant>\n\t\t\t<Catalog>Items</Catalog>\n\t\t\t<Document>Order</Document>\n\t\t\t<ChartOfAccounts>Accounts</ChartOfAccounts>\n\t\t\t<Catalog>Владельцы</Catalog>\n\t\t\t<DocumentJournal>Журнал</DocumentJournal>\n\t\t\t<ChartOfCalculationTypes>ВидыРасчета</ChartOfCalculationTypes>\n\t\t\t<EventSubscription>ПриЗаписи</EventSubscription>\n\t\t\t<CalculationRegister>Payroll</CalculationRegister>\n\t\t\t<Report>ParityReport</Report>\n\t\t\t<Role>SalesReader</Role>\n\t\t\t<Subsystem>Sales</Subsystem>\n\t\t\t<XDTOPackage>EnterpriseData_1_17_3</XDTOPackage>\n\t\t\t<CommonModule>РеактивныйСервер</CommonModule>\n\t\t\t<ScheduledJob>MonthClose</ScheduledJob>\n\t\t\t<HTTPService>ExternalAPI</HTTPService>\n\t\t\t<WebService>Exchange</WebService>\n\t\t",
                ),
            )
            .unwrap();
        let catalog = with_root_version(&fixture_text(
            "platform_8_3_27/meta_info/edge/catalog-child-kinds.xml",
        ))
        .replace(
            "<TabularSection><Properties><Name>Lines</Name></Properties><ChildObjects/></TabularSection>",
            "<TabularSection><Properties><Name>Lines</Name></Properties><ChildObjects><Attribute><Properties><Name>Quantity</Name><Type><v8:Type>xs:decimal</v8:Type></Type></Properties></Attribute></ChildObjects></TabularSection>",
        )
        // Свойства корня каталога: без них узел метаданных нечем проверить.
        .replace(
            "<Properties><Name>Items</Name></Properties>",
            concat!(
                "<Properties xmlns:xr=\"http://v8.1c.ru/8.3/xcf/readable\"",
                " xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\"",
                " xmlns:xs=\"http://www.w3.org/2001/XMLSchema\">",
                "<Name>Items</Name><Hierarchical>true</Hierarchical>",
                "<CodeType>String</CodeType><CodeLength>11</CodeLength>",
                "<Owners><xr:Item xsi:type=\"xr:MDObjectRef\">Catalog.Владельцы</xr:Item></Owners>",
                "<InputByString><xr:Field>Catalog.Items.Attribute.Code</xr:Field></InputByString>",
                "<DataLockFields><xr:Field>Catalog.Items.Attribute.Code</xr:Field></DataLockFields>",
                // Характеристика названа парой источников, а не именем.
                "<Characteristics>",
                "<xr:Characteristic>",
                "<xr:CharacteristicTypes from=\"Catalog.Items.TabularSection.Lines.Attribute.Quantity\">",
                "<xr:KeyField>Catalog.Items.Attribute.Code</xr:KeyField>",
                "<xr:TypesFilterField>Catalog.Items.Attribute.Code</xr:TypesFilterField>",
                "<xr:TypesFilterValue xsi:type=\"xs:string\">Catalog_Items</xr:TypesFilterValue>",
                "<xr:DataPathField>-1</xr:DataPathField>",
                "<xr:MultipleValuesUseField>-1</xr:MultipleValuesUseField>",
                "</xr:CharacteristicTypes>",
                "<xr:CharacteristicValues from=\"Catalog.Items\">",
                "<xr:ObjectField>Catalog.Items.Attribute.Code</xr:ObjectField>",
                "<xr:TypeField>Catalog.Items.Attribute.Code</xr:TypeField>",
                "<xr:ValueField>Catalog.Items.Attribute.Code</xr:ValueField>",
                "<xr:MultipleValuesKeyField>-1</xr:MultipleValuesKeyField>",
                "<xr:MultipleValuesOrderField>-1</xr:MultipleValuesOrderField>",
                "</xr:CharacteristicValues>",
                "</xr:Characteristic></Characteristics></Properties>",
            ),
        );
        write(&source.join("Catalogs/Items.xml"), &catalog);
        // Остальные источники ссылок: владелец, журнал, базовые виды расчёта и
        // источник события подписки. Правило обещает всякую ссылку, и проверить
        // его можно только всеми её источниками.
        for (relative, body) in [
            (
                "Catalogs/Владельцы.xml",
                r#"<Catalog uuid="99999999-9999-4999-8999-999999999991"><Properties><Name>Владельцы</Name></Properties><ChildObjects/></Catalog>"#,
            ),
            (
                "DocumentJournals/Журнал.xml",
                r#"<DocumentJournal uuid="99999999-9999-4999-8999-999999999992"><Properties><Name>Журнал</Name><RegisteredDocuments><xr:Item xsi:type="xr:MDObjectRef">Document.Order</xr:Item></RegisteredDocuments></Properties><ChildObjects/></DocumentJournal>"#,
            ),
            (
                "ChartsOfCalculationTypes/ВидыРасчета.xml",
                r#"<ChartOfCalculationTypes uuid="99999999-9999-4999-8999-999999999993"><Properties><Name>ВидыРасчета</Name><BaseCalculationTypes><xr:Item xsi:type="xr:MDObjectRef">ChartOfCalculationTypes.ВидыРасчета</xr:Item></BaseCalculationTypes></Properties><ChildObjects/></ChartOfCalculationTypes>"#,
            ),
            (
                "EventSubscriptions/ПриЗаписи.xml",
                r#"<EventSubscription uuid="99999999-9999-4999-8999-999999999994"><Properties><Name>ПриЗаписи</Name><Source><v8:Type>cfg:CatalogObject.Items</v8:Type></Source><Event>BeforeWrite</Event><Handler>CommonModule.РеактивныйСервер.ПриЗаписи</Handler></Properties></EventSubscription>"#,
            ),
        ] {
            write(
                &source.join(relative),
                &format!(
                    concat!(
                        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n",
                        "<MetaDataObject xmlns=\"http://v8.1c.ru/8.3/MDClasses\"",
                        " xmlns:v8=\"http://v8.1c.ru/8.1/data/core\"",
                        " xmlns:cfg=\"http://v8.1c.ru/8.1/data/enterprise/current-config\"",
                        " xmlns:xr=\"http://v8.1c.ru/8.3/xcf/readable\"",
                        " xmlns:xs=\"http://www.w3.org/2001/XMLSchema\"",
                        " xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\"",
                        " version=\"2.20\">{}</MetaDataObject>"
                    ),
                    body
                ),
            );
        }
        // План счетов со стандартной табличной частью: у неё есть имя, и она
        // адресуется, а внутри — своя последовательность.
        write(
            &source.join("ChartsOfAccounts/Accounts.xml"),
            r#"<?xml version="1.0" encoding="UTF-8"?>
<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" xmlns:xr="http://v8.1c.ru/8.3/xcf/readable" version="2.20">
  <ChartOfAccounts uuid="77777777-7777-4777-8777-777777777778">
    <Properties>
      <Name>Accounts</Name>
      <StandardTabularSections>
        <xr:StandardTabularSection name="ExtDimensionTypes">
          <xr:Synonym/><xr:Comment/><xr:ToolTip/>
          <xr:FillChecking>DontCheck</xr:FillChecking>
          <xr:StandardAttributes>
            <xr:StandardAttribute name="ExtDimensionType"/>
          </xr:StandardAttributes>
        </xr:StandardTabularSection>
      </StandardTabularSections>
    </Properties>
    <ChildObjects/>
  </ChartOfAccounts>
</MetaDataObject>"#,
        );
        // Предопределённые элементы справочника: писатель у них есть, и теперь
        // есть читатель.
        write(
            &source.join("Catalogs/Items/Ext/Predefined.xml"),
            r#"<?xml version="1.0" encoding="UTF-8"?>
<PredefinedData xmlns="http://v8.1c.ru/8.3/xcf/predef" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" xsi:type="CatalogPredefinedItems" version="2.20">
  <Item id="c7d2e6fc-3824-4b56-b4be-ae6be4944c0e">
    <Name>Основной</Name>
    <Code/>
    <Description>Основной элемент</Description>
    <IsFolder>false</IsFolder>
  </Item>
</PredefinedData>"#,
        );
        // Документ со ссылками наружу: движение по регистру и основание.
        write(
            &source.join("Documents/Order.xml"),
            r#"<?xml version="1.0" encoding="UTF-8"?>
<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" xmlns:xr="http://v8.1c.ru/8.3/xcf/readable" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" version="2.20">
  <Document uuid="88888888-8888-4888-8888-888888888888">
    <Properties>
      <Name>Order</Name>
      <RegisterRecords>
        <xr:Item xsi:type="xr:MDObjectRef">CalculationRegister.Payroll</xr:Item>
      </RegisterRecords>
      <BasedOn>
        <xr:Item xsi:type="xr:MDObjectRef">Catalog.Items</xr:Item>
      </BasedOn>
    </Properties>
    <ChildObjects/>
  </Document>
</MetaDataObject>"#,
        );
        // Три вида с собственными данными: обработчик, расписание и тип.
        for (relative, fixture) in [
            ("Constants/MainCurrency.xml", "constant-type.xml"),
            (
                "CalculationRegisters/Payroll.xml",
                "calculation-register.xml",
            ),
            ("ScheduledJobs/MonthClose.xml", "scheduled-job.xml"),
            ("HTTPServices/ExternalAPI.xml", "http-service.xml"),
            ("WebServices/Exchange.xml", "web-service.xml"),
        ] {
            write(
                &source.join(relative),
                &with_root_version(&fixture_text(&format!(
                    "platform_8_3_27/meta_info/edge/{fixture}"
                ))),
            );
        }
        write(
            &source.join("Catalogs/Items/Forms/ItemForm.xml"),
            r#"<?xml version="1.0" encoding="UTF-8"?><MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Form uuid="10000000-0000-4000-8000-000000000031"><Properties><Name>ItemForm</Name><FormType>Managed</FormType></Properties></Form></MetaDataObject>"#,
        );
        write(
            &source.join("Catalogs/Items/Forms/ItemForm/Ext/Form.xml"),
            r#"<?xml version="1.0" encoding="UTF-8"?><Form xmlns="http://v8.1c.ru/8.3/xcf/logform" version="2.20"><ChildItems/></Form>"#,
        );
        write(
            &source.join("Catalogs/Items/Templates/Print.xml"),
            r#"<?xml version="1.0" encoding="UTF-8"?><MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Template uuid="10000000-0000-4000-8000-000000000032"><Properties><Name>Print</Name><TemplateType>SpreadsheetDocument</TemplateType></Properties></Template></MetaDataObject>"#,
        );
        copy_fixture(
            "platform_8_3_27/mxl/Template.xml",
            &source.join("Catalogs/Items/Templates/Print/Ext/Template.xml"),
        );
        write(
            &source.join("Catalogs/Items/Commands/Refresh/Ext/CommandModule.bsl"),
            "&AtClient\nProcedure CommandProcessing(CommandParameter, CommandExecuteParameters)\nEndProcedure\n",
        );
        let report = fixture_text("unica_mcp_script_parity/form-remove/ParityReport.xml");
        fs::create_dir_all(source.join("Reports")).unwrap();
        fs::write(
                source.join("Reports/ParityReport.xml"),
                replace_child_objects(
                    &report,
                    "\n\t\t\t<Form>MainForm</Form>\n\t\t\t<Template>MainSchema</Template>\n\t\t\t<Template>Print</Template>\n\t\t",
                ),
            )
            .unwrap();
        copy_fixture(
            "unica_mcp_script_parity/form-remove/ParityReport/Forms/MainForm.xml",
            &source.join("Reports/ParityReport/Forms/MainForm.xml"),
        );
        let form = r#"<?xml version="1.0" encoding="utf-8"?>
<Form xmlns="http://v8.1c.ru/8.3/xcf/logform" version="2.20">
  <ChildItems>
    <Table name="Goods">
      <ChildItems>
        <InputField name="Quantity">
          <Events><Event name="OnChange">QuantityOnChange</Event></Events>
        </InputField>
      </ChildItems>
    </Table>
  </ChildItems>
  <Events><Event name="OnOpen">OnOpen</Event></Events>
</Form>"#;
        write(
            &source.join("Reports/ParityReport/Forms/MainForm/Ext/Form.xml"),
            form,
        );
        write(
            &source.join("Reports/ParityReport/Forms/MainForm/Ext/Form/Module.bsl"),
            "&AtClient\nProcedure OnOpen()\nEndProcedure\n\n&AtClient\nProcedure QuantityOnChange()\nEndProcedure\n",
        );
        write(
            &source.join("Roles/SalesReader.xml"),
            &with_root_version(&fixture_text(
                "unica_mcp_script_parity/role-info/SalesReader.xml",
            )),
        );
        write(
            &source.join("Roles/SalesReader/Ext/Rights.xml"),
            &fixture_text("unica_mcp_script_parity/role-info/SalesReader/Ext/Rights.xml")
                .replace("version=\"2.17\"", "version=\"2.20\""),
        );
        let subsystem = r#"<?xml version="1.0" encoding="utf-8"?>
<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" xmlns:v8="http://v8.1c.ru/8.1/data/core" xmlns:xr="http://v8.1c.ru/8.3/xcf/readable" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" version="2.20">
  <Subsystem><Properties><Name>Sales</Name><Synonym><v8:item><v8:lang>ru</v8:lang><v8:content>Продажи</v8:content></v8:item></Synonym><IncludeInCommandInterface>true</IncludeInCommandInterface><UseOneCommand>false</UseOneCommand><Content><xr:Item xsi:type="xr:MDObjectRef">Catalog.Items</xr:Item></Content></Properties><ChildObjects/></Subsystem>
</MetaDataObject>"#;
        write(&source.join("Subsystems/Sales.xml"), subsystem);
        copy_fixture(
            "unica_mcp_script_parity/meta-remove/Subsystems/Sales/Ext/CommandInterface.xml",
            &source.join("Subsystems/Sales/Ext/CommandInterface.xml"),
        );
        copy_fixture(
            "unica_mcp_script_parity/template-remove/ParityReport/Templates/MainSchema.xml",
            &source.join("Reports/ParityReport/Templates/MainSchema.xml"),
        );
        let dcs = fixture_text("unica_mcp_script_parity/dcs-validate/BadPrefix.xml")
            .replace(
                "xmlns:bad=\"http://example.com\">bad:CatalogRef.X",
                ">xs:string",
            )
            .replace(
                "\n</DataCompositionSchema>",
                "\n\t<parameter><name>Period</name><valueType><v8:Type>xs:dateTime</v8:Type></valueType></parameter>\n</DataCompositionSchema>",
            );
        write(
            &source.join("Reports/ParityReport/Templates/MainSchema/Ext/Template.xml"),
            &dcs,
        );
        let print_descriptor = fixture_text(
            "unica_mcp_script_parity/template-remove/ParityReport/Templates/MainSchema.xml",
        )
        .replace("MainSchema", "Print")
        .replace("DataCompositionSchema", "SpreadsheetDocument");
        write(
            &source.join("Reports/ParityReport/Templates/Print.xml"),
            &print_descriptor,
        );
        let mxl = fixture_text("platform_8_3_27/mxl/Template.xml")
            .replace(
                "\n\t<templateMode>true</templateMode>",
                "\n\t<namedItem xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\" xsi:type=\"NamedItemCells\"><name>Header</name><area><type>Rows</type><beginRow>0</beginRow><endRow>0</endRow><beginColumn>-1</beginColumn><endColumn>-1</endColumn></area></namedItem>\n\t<templateMode>true</templateMode>",
            )
            .replace(
                "\n\t\t\t<empty>true</empty>",
                "\n\t\t\t<c><c><f>0</f><parameter>Title</parameter></c></c><c><c><f>1</f><tl><content>Sales report</content></tl></c></c><c><c><f>2</f><tl><content>Period: [Since]</content></tl></c></c>",
            );
        write(
            &source.join("Reports/ParityReport/Templates/Print/Ext/Template.xml"),
            &mxl,
        );
        copy_fixture(
            "xdto/enterprise-data-minimal/XDTOPackages/EnterpriseData_1_17_3.xml",
            &source.join("XDTOPackages/EnterpriseData_1_17_3.xml"),
        );
        copy_fixture(
            "xdto/enterprise-data-minimal/XDTOPackages/EnterpriseData_1_17_3/Ext/Package.bin",
            &source.join("XDTOPackages/EnterpriseData_1_17_3/Ext/Package.bin"),
        );
        copy_fixture(
            "platform_8_3_27/support-edit-bin-only/src/CommonModules/РеактивныйСервер.xml",
            &source.join("CommonModules/РеактивныйСервер.xml"),
        );
        copy_fixture(
                "platform_8_3_27/support-edit-bin-only/src/CommonModules/РеактивныйСервер/Ext/Module.bsl",
                &source.join("CommonModules/РеактивныйСервер/Ext/Module.bsl"),
            );
        let common_module_path = source.join("CommonModules/РеактивныйСервер/Ext/Module.bsl");
        let mut common_module = fs::read_to_string(&common_module_path).unwrap();
        common_module.push_str("\nПроцедура InternalService()\nКонецПроцедуры\n");
        fs::write(common_module_path, common_module).unwrap();
        let canonical_root = fs::canonicalize(root.path()).unwrap();
        let source = fs::canonicalize(&source).unwrap();
        let context = WorkspaceContext {
            cwd: canonical_root.clone(),
            workspace_root: canonical_root,
            cache_root: cache,
            workspace_epoch: 1,
        };
        Self {
            root,
            context,
            source,
            cancellation: CancellationToken::new(),
        }
    }

    pub(super) fn read_authority(&self) -> LogicalViewReadAuthority<'_> {
        let source_root = Arc::new(RetainedDirectoryCapability::open(&self.source).unwrap());
        LogicalViewReadAuthority::new(
            &self.cancellation,
            "main",
            "actor-fixture-main",
            SourceSetKind::Configuration,
            source_root,
            PlatformProfile::v8_3_27(),
        )
    }

    pub(super) fn operation_read_authority(&self) -> LogicalViewReadAuthority<'_> {
        let root = Arc::new(RetainedDirectoryCapability::open(&self.source).unwrap());
        let deadline = ProviderDeadline::no_deadline();
        LogicalViewReadAuthority::with_read_authority(
            &self.cancellation,
            ProviderReadAuthority::new(
                "main",
                "actor-fixture-main",
                SourceSetKind::Configuration,
                root,
            ),
            PlatformProfile::v8_3_27(),
            deadline,
        )
    }

    pub(super) fn view_service(&self) -> ViewService<LogicalViewReadAuthority<'_>> {
        ViewService::new(self.read_authority(), ViewCursorStore::default())
    }

    /// Тот же набор, объявленный расширением: заимствование живёт только здесь.
    pub(super) fn extension_read_authority(&self) -> LogicalViewReadAuthority<'_> {
        let source_root = Arc::new(RetainedDirectoryCapability::open(&self.source).unwrap());
        LogicalViewReadAuthority::new(
            &self.cancellation,
            "main",
            "actor-fixture-extension-borrowing",
            SourceSetKind::Extension,
            source_root,
            PlatformProfile::v8_3_27(),
        )
    }

    pub(super) fn extension_view_service(&self) -> ViewService<LogicalViewReadAuthority<'_>> {
        ViewService::new(self.extension_read_authority(), ViewCursorStore::default())
    }

    /// Платформенная форма заимствования: `Adopted` в свойствах,
    /// `ExtendedConfigurationObject` с UUID родителя после имени и, когда
    /// свойство перекрыто, запись `xr:PropertyState` в `InternalInfo`.
    pub(super) fn borrow_catalog(&self, relative: &str, overridden: Option<&str>) {
        let path = self.source.join(relative);
        let text = fs::read_to_string(&path).unwrap();
        let properties = text.find("<Properties>").unwrap();
        let after_open = properties + "<Properties>".len();
        let mut patched = format!(
            "{}<ObjectBelonging>Adopted</ObjectBelonging>{}",
            &text[..after_open],
            &text[after_open..]
        );
        let name_end = patched.find("</Name>").unwrap() + "</Name>".len();
        patched = format!(
            "{}<ExtendedConfigurationObject>11111111-1111-4111-8111-111111111111</ExtendedConfigurationObject>{}",
            &patched[..name_end],
            &patched[name_end..]
        );
        if let Some(property) = overridden {
            let properties = patched.find("<Properties>").unwrap();
            patched = format!(
                "{}<InternalInfo><xr:PropertyState><xr:Property>{property}</xr:Property><xr:State>Extended</xr:State></xr:PropertyState></InternalInfo>{}",
                &patched[..properties],
                &patched[properties..]
            );
        }
        write(&path, &patched);
    }

    pub(super) fn install_main_form_sources(&self, form_xml: &str, module_bsl: &str) {
        write(
            &self
                .source
                .join("Reports/ParityReport/Forms/MainForm/Ext/Form.xml"),
            form_xml,
        );
        write(
            &self
                .source
                .join("Reports/ParityReport/Forms/MainForm/Ext/Form/Module.bsl"),
            module_bsl,
        );
    }

    pub(super) fn install_module_matrix_sources(&self) {
        let configuration_path = self.source.join("Configuration.xml");
        let configuration = fs::read_to_string(&configuration_path).unwrap();
        let registrations = [
            ("Document", "Заказ"),
            ("Document", "ЕщеНеВыгружен"),
            ("InformationRegister", "Цены"),
            ("Constant", "ОсновнаяВалюта"),
            ("CommonModule", "ЗаказыСервер"),
            ("CommonForm", "Подбор"),
            ("CommonCommand", "ОткрытьНастройки"),
            ("HTTPService", "API"),
            ("WebService", "Обмен"),
            ("IntegrationService", "Шина"),
            ("Bot", "Помощник"),
            ("WebSocketClient", "Телефония"),
        ]
        .into_iter()
        .map(|(kind, name)| format!("\n\t\t\t<{kind}>{name}</{kind}>"))
        .collect::<String>();
        fs::write(
            &configuration_path,
            configuration.replacen(
                "</ChildObjects>",
                &format!("{registrations}\n\t\t</ChildObjects>"),
                1,
            ),
        )
        .unwrap();

        write_metadata_owner(
            &self.source,
            "Documents",
            "Document",
            "Заказ",
            "<Form>ФормаДокумента</Form><Command uuid=\"10000000-0000-4000-8000-000000000022\"><Properties><Name>ПровестиИЗакрыть</Name></Properties></Command>",
        );
        write_metadata_owner(&self.source, "Documents", "Document", "ЕщеНеВыгружен", "");
        write_metadata_owner(
            &self.source,
            "InformationRegisters",
            "InformationRegister",
            "Цены",
            "",
        );
        write_metadata_owner(&self.source, "Constants", "Constant", "ОсновнаяВалюта", "");
        let common = fixture_text(
            "platform_8_3_27/support-edit-bin-only/src/CommonModules/РеактивныйСервер.xml",
        )
        .replace("РеактивныйСервер", "ЗаказыСервер");
        write(&self.source.join("CommonModules/ЗаказыСервер.xml"), &common);
        write_metadata_owner(&self.source, "CommonForms", "CommonForm", "Подбор", "");
        write(
            &self.source.join("CommonForms/Подбор/Ext/Form.xml"),
            r#"<?xml version="1.0" encoding="UTF-8"?><Form xmlns="http://v8.1c.ru/8.3/xcf/logform" version="2.20"><ChildItems/></Form>"#,
        );
        write_metadata_owner(
            &self.source,
            "CommonCommands",
            "CommonCommand",
            "ОткрытьНастройки",
            "",
        );
        for (directory, kind, name) in [
            ("HTTPServices", "HTTPService", "API"),
            ("WebServices", "WebService", "Обмен"),
            ("IntegrationServices", "IntegrationService", "Шина"),
            ("Bots", "Bot", "Помощник"),
        ] {
            write_metadata_owner(&self.source, directory, kind, name, "");
        }
        write(
            &self.source.join("Documents/Заказ/Forms/ФормаДокумента.xml"),
            r#"<?xml version="1.0" encoding="UTF-8"?><MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Form uuid="10000000-0000-4000-8000-000000000021"><Properties><Name>ФормаДокумента</Name><FormType>Managed</FormType></Properties></Form></MetaDataObject>"#,
        );
        write(
            &self
                .source
                .join("Documents/Заказ/Forms/ФормаДокумента/Ext/Form.xml"),
            r#"<?xml version="1.0" encoding="UTF-8"?><Form xmlns="http://v8.1c.ru/8.3/xcf/logform" version="2.20"><ChildItems/></Form>"#,
        );
        write(
            &self
                .source
                .join("Documents/Заказ/Commands/ПровестиИЗакрыть/Ext/CommandModule.bsl"),
            "&AtClient\nProcedure CommandProcessing(CommandParameter, CommandExecuteParameters)\nEndProcedure\n",
        );
    }

    pub(super) fn install_accepted_profile_sources(&self) {
        let configuration_path = self.source.join("Configuration.xml");
        let configuration = fs::read_to_string(&configuration_path).unwrap();
        fs::write(
            configuration_path,
            configuration.replace(
                "</ChildObjects>",
                "\n<Document>Заказ</Document>\n<Role>Кладовщик</Role>\n<Report>Продажи</Report>\n</ChildObjects>",
            ),
        )
        .unwrap();

        write(
            &self.source.join("Documents/Заказ.xml"),
            r#"<?xml version="1.0" encoding="UTF-8"?>
<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" xmlns:v8="http://v8.1c.ru/8.1/data/core" xmlns:xs="http://www.w3.org/2001/XMLSchema" version="2.20">
  <Document uuid="10000000-0000-4000-8000-000000000001">
    <Properties><Name>Заказ</Name></Properties>
    <ChildObjects>
      <Attribute><Properties><Name>Контрагент</Name><Type><v8:Type>xs:string</v8:Type></Type></Properties></Attribute>
      <TabularSection><Properties><Name>Товары</Name></Properties><ChildObjects>
        <Attribute><Properties><Name>Количество</Name><Type><v8:Type>xs:decimal</v8:Type></Type></Properties></Attribute>
      </ChildObjects></TabularSection>
      <Form>ФормаДокумента</Form>
    </ChildObjects>
  </Document>
</MetaDataObject>"#,
        );
        write(
            &self.source.join("Documents/Заказ/Forms/ФормаДокумента.xml"),
            r#"<?xml version="1.0" encoding="UTF-8"?>
<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Form uuid="10000000-0000-4000-8000-000000000002"><Properties><Name>ФормаДокумента</Name><FormType>Managed</FormType></Properties></Form></MetaDataObject>"#,
        );
        write(
            &self
                .source
                .join("Documents/Заказ/Forms/ФормаДокумента/Ext/Form.xml"),
            r#"<?xml version="1.0" encoding="UTF-8"?>
<Form xmlns="http://v8.1c.ru/8.3/xcf/logform" xmlns:v8="http://v8.1c.ru/8.1/data/core" xmlns:xs="http://www.w3.org/2001/XMLSchema" version="2.20">
  <ChildItems>
    <InputField name="Склад" id="1"/>
    <Table name="Товары" id="2"><ChildItems><InputField name="Количество" id="3"><Events><Event name="Change">КоличествоИзменение</Event></Events></InputField></ChildItems></Table>
  </ChildItems>
  <Attributes><Attribute name="Объект" id="1"><Type><v8:Type>xs:string</v8:Type></Type></Attribute></Attributes>
  <Parameters><Parameter name="Режим"><Type><v8:Type>xs:string</v8:Type></Type></Parameter></Parameters>
  <Commands><Command name="Пересчитать" id="1"/></Commands>
</Form>"#,
        );

        write(
            &self.source.join("Roles/Кладовщик.xml"),
            r#"<?xml version="1.0" encoding="UTF-8"?>
<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Role uuid="10000000-0000-4000-8000-000000000003"><Properties><Name>Кладовщик</Name></Properties></Role></MetaDataObject>"#,
        );
        write(
            &self.source.join("Roles/Кладовщик/Ext/Rights.xml"),
            r#"<?xml version="1.0" encoding="UTF-8"?>
<Rights xmlns="http://v8.1c.ru/8.2/roles" version="2.20"><object><name>Catalog.Товары</name><right><name>Read</name><value>true</value><restrictionByCondition><condition>Товары.Владелец = &amp;ТекущийПользователь</condition></restrictionByCondition></right></object></Rights>"#,
        );

        let report = fixture_text("unica_mcp_script_parity/form-remove/ParityReport.xml")
            .replace("ParityReport", "Продажи");
        write(
            &self.source.join("Reports/Продажи.xml"),
            &replace_child_objects(
                &report,
                "<Template>ОсновнаяСхема</Template><Template>Печать</Template>",
            ),
        );
        let schema_descriptor = fixture_text(
            "unica_mcp_script_parity/template-remove/ParityReport/Templates/MainSchema.xml",
        )
        .replace("MainSchema", "ОсновнаяСхема");
        write(
            &self
                .source
                .join("Reports/Продажи/Templates/ОсновнаяСхема.xml"),
            &schema_descriptor,
        );
        let dcs = fixture_text("unica_mcp_script_parity/dcs-validate/BadPrefix.xml")
            .replace("MainData", "Продажи")
            .replace("Code", "Сумма")
            .replace(
                "xmlns:bad=\"http://example.com\">bad:CatalogRef.X",
                ">xs:decimal",
            )
            .replace(
                "\n</DataCompositionSchema>",
                "\n<parameter><name>Период</name><valueType><v8:Type>xs:dateTime</v8:Type></valueType></parameter>\n</DataCompositionSchema>",
            );
        write(
            &self
                .source
                .join("Reports/Продажи/Templates/ОсновнаяСхема/Ext/Template.xml"),
            &dcs,
        );
        let print_descriptor = schema_descriptor
            .replace("ОсновнаяСхема", "Печать")
            .replace("DataCompositionSchema", "SpreadsheetDocument");
        write(
            &self.source.join("Reports/Продажи/Templates/Печать.xml"),
            &print_descriptor,
        );
        let mxl = fixture_text("platform_8_3_27/mxl/Template.xml")
            .replace(
                "\n\t<templateMode>true</templateMode>",
                "\n\t<namedItem xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\" xsi:type=\"NamedItemCells\"><name>Шапка</name><area><type>Rows</type><beginRow>0</beginRow><endRow>0</endRow><beginColumn>-1</beginColumn><endColumn>-1</endColumn></area></namedItem>\n\t<templateMode>true</templateMode>",
            )
            .replace(
                "\n\t\t\t<empty>true</empty>",
                "\n\t\t\t<c><c><f>0</f><parameter>Заголовок</parameter></c></c>",
            );
        write(
            &self
                .source
                .join("Reports/Продажи/Templates/Печать/Ext/Template.xml"),
            &mxl,
        );
    }
}

pub(super) fn fixture_path(relative: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures")
        .join(relative)
}

pub(super) fn fixture_text(relative: &str) -> String {
    fs::read_to_string(fixture_path(relative)).unwrap()
}

pub(super) fn copy_fixture(relative: &str, destination: &Path) {
    fs::create_dir_all(destination.parent().unwrap()).unwrap();
    fs::copy(fixture_path(relative), destination).unwrap();
}

pub(super) fn write(path: &Path, text: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
}

pub(super) fn replace_child_objects(document: &str, body: &str) -> String {
    let start = document.find("<ChildObjects>").unwrap();
    let end = document.find("</ChildObjects>").unwrap() + "</ChildObjects>".len();
    format!(
        "{}<ChildObjects>{body}</ChildObjects>{}",
        &document[..start],
        &document[end..]
    )
}

fn with_root_version(document: &str) -> String {
    let start = document.find("<MetaDataObject").unwrap();
    let end = document[start..].find('>').unwrap() + start;
    if document[start..end].contains("version=") {
        return document.to_string();
    }
    format!("{} version=\"2.20\"{}", &document[..end], &document[end..])
}

/// Описание из выгрузки платформы, сверенное с `manifest.json` опыта: тест
/// читает ровно ту форму, которую выгрузила платформа.
fn platform_borrow_fixture(name: &str) -> String {
    use sha2::{Digest, Sha256};
    let manifest: serde_json::Value =
        serde_json::from_str(&fixture_text("platform_8_3_27/cfe_borrow/manifest.json")).unwrap();
    let text = fixture_text(&format!("platform_8_3_27/cfe_borrow/{name}"));
    let digest = Sha256::digest(text.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    assert_eq!(
        manifest["files"][name]["sha256"].as_str(),
        Some(digest.as_str()),
        "{name} differs from the platform export recorded in the manifest"
    );
    text
}

/// Дерево из выгрузки опыта #1381: роль, общая форма, обработка с формой и
/// двумя макетами и пакет XDTO заимствованы без своих файлов содержимого.
pub(super) fn install_inherited_objects(fixture: &RealReaderFixture) {
    let configuration = fs::read_to_string(fixture.source.join("Configuration.xml")).unwrap();
    write(
        &fixture.source.join("Configuration.xml"),
        &configuration.replace(
            "</ChildObjects>",
            "<CommonForm>ОбщаяФорма</CommonForm><DataProcessor>Обработка</DataProcessor><XDTOPackage>ПакетОбмена</XDTOPackage></ChildObjects>",
        ),
    );
    fs::remove_dir_all(fixture.source.join("Roles/SalesReader")).unwrap();
    for (relative, name) in [
        ("Roles/SalesReader.xml", "extension-role-inherited.xml"),
        (
            "CommonForms/ОбщаяФорма.xml",
            "extension-common-form-inherited.xml",
        ),
        (
            "DataProcessors/Обработка.xml",
            "extension-data-processor.xml",
        ),
        (
            "DataProcessors/Обработка/Forms/Форма.xml",
            "extension-object-form-inherited.xml",
        ),
        (
            "DataProcessors/Обработка/Templates/Макет.xml",
            "extension-template-spreadsheet-inherited.xml",
        ),
        (
            "DataProcessors/Обработка/Templates/СхемаКомпоновки.xml",
            "extension-template-schema-inherited.xml",
        ),
        (
            "XDTOPackages/ПакетОбмена.xml",
            "extension-xdto-package-inherited.xml",
        ),
    ] {
        write(
            &fixture.source.join(relative),
            &platform_borrow_fixture(name),
        );
    }
}
