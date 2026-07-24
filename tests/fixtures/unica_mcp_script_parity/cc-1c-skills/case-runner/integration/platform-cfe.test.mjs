// platform-cfe.test.mjs — Integration test: load CFE extension into 1C platform
// Requires: 1C platform (1cv8.exe) via .v8-project.json
// Steps: build config → build extension → db-create → load config → load extension → update

export const name = 'Загрузка расширения в базу с конфигурацией';
export const setup = 'none';
export const requiresPlatform = true;

export const steps = [
  // ── 1. Build minimal base config ──
  {
    name: 'cf-init: базовая конфигурация',
    script: 'cf-init/scripts/cf-init',
    args: { '-Name': 'БазаДляРасширения', '-OutputDir': '{workDir}/config' },
  },
  {
    name: 'meta-compile: Справочник Контрагенты',
    script: 'meta-compile/scripts/meta-compile',
    input: { type: 'Catalog', name: 'Контрагенты', codeLength: 9, descriptionLength: 100 },
    args: { '-JsonPath': '{inputFile}', '-OutputDir': '{workDir}/config' },
  },
  {
    name: 'cf-edit: регистрация каталога',
    script: 'cf-edit/scripts/cf-edit',
    input: [
      { operation: 'add-childObject', value: 'Catalog.Контрагенты' },
    ],
    args: { '-ConfigPath': '{workDir}/config', '-DefinitionFile': '{inputFile}' },
  },

  {
    name: 'form-add: форма справочника',
    script: 'form-add/scripts/form-add',
    args: {
      '-ObjectPath': '{workDir}/config/Catalogs/Контрагенты',
      '-FormName': 'ФормаЭлемента',
      '-Purpose': 'Object',
    },
  },
  {
    name: 'form-compile: наполнение формы справочника',
    script: 'form-compile/scripts/form-compile',
    input: {
      elements: [
        { id: 'Код', type: 'input', path: 'Object.Code', title: 'Код' },
        { id: 'Наименование', type: 'input', path: 'Object.Description', title: 'Наименование' },
      ],
    },
    args: { '-OutputPath': '{workDir}/config/Catalogs/Контрагенты/Forms/ФормаЭлемента/Ext/Form.xml', '-JsonPath': '{inputFile}' },
  },

  // ── 2. Build extension ──
  {
    name: 'cfe-init: расширение',
    script: 'cfe-init/scripts/cfe-init',
    args: {
      '-Name': 'ТестРасширение',
      '-OutputDir': '{workDir}/ext',
      '-ConfigPath': '{workDir}/config',
    },
  },
  {
    name: 'cfe-borrow: заимствование Catalog.Контрагенты',
    script: 'cfe-borrow/scripts/cfe-borrow',
    args: {
      '-ExtensionPath': '{workDir}/ext',
      '-ConfigPath': '{workDir}/config',
      '-Object': 'Catalog.Контрагенты',
    },
  },

  // ── 3. Create DB, load config ──
  {
    name: 'db-create: создание ИБ',
    script: 'db-create/scripts/db-create',
    args: { '-V8Path': '{v8path}', '-InfoBasePath': '{workDir}/testdb' },
  },
  {
    name: 'db-load-xml: загрузка конфигурации',
    script: 'db-load-xml/scripts/db-load-xml',
    args: {
      '-V8Path': '{v8path}',
      '-InfoBasePath': '{workDir}/testdb',
      '-ConfigDir': '{workDir}/config',
    },
  },
  {
    name: 'db-update: обновление БД (конфигурация)',
    script: 'db-update/scripts/db-update',
    args: { '-V8Path': '{v8path}', '-InfoBasePath': '{workDir}/testdb' },
  },

  // ── 4. Load extension ──
  {
    name: 'db-load-xml: загрузка расширения',
    script: 'db-load-xml/scripts/db-load-xml',
    args: {
      '-V8Path': '{v8path}',
      '-InfoBasePath': '{workDir}/testdb',
      '-ConfigDir': '{workDir}/ext',
      '-Extension': 'ТестРасширение',
    },
  },
  {
    name: 'db-update: обновление БД (расширение)',
    script: 'db-update/scripts/db-update',
    args: {
      '-V8Path': '{v8path}',
      '-InfoBasePath': '{workDir}/testdb',
      '-Extension': 'ТестРасширение',
    },
  },
];
