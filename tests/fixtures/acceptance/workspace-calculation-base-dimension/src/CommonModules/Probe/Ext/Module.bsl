Функция Run() Экспорт
 Salary = ПланыВидовРасчета.Kinds.НайтиПоКоду("SALARY");
 Если Salary.Пустая() Тогда
  Obj = ПланыВидовРасчета.Kinds.СоздатьВидРасчета();
  Obj.Код = "SALARY"; Obj.Наименование = "Salary"; Obj.Записать(); Salary = Obj.Ссылка;
  Obj = ПланыВидовРасчета.Kinds.СоздатьВидРасчета();
  Obj.Код = "BONUS"; Obj.Наименование = "Bonus";
  Obj.БазовыеВидыРасчета.Добавить().ВидРасчета = Salary;
  Obj.Записать();
 КонецЕсли;
 Bonus = ПланыВидовРасчета.Kinds.НайтиПоКоду("BONUS");
 BaseDoc = Документы.Orders.НайтиПоНомеру("BASE");
 Если BaseDoc.Пустая() Тогда
  Doc = Документы.Orders.СоздатьДокумент(); Doc.Номер = "BASE"; Doc.Дата = Дата(2026,1,10); Doc.Записать(); BaseDoc = Doc.Ссылка;
  Set = РегистрыРасчета.Accruals.СоздатьНаборЗаписей(); Set.Отбор.Регистратор.Установить(BaseDoc);
  Для Каждого Data Из СтрРазделить("Alice:A:100,Alice:B:999,Bob:A:777", ",") Цикл
   Parts = СтрРазделить(Data, ":"); Row = Set.Добавить(); Row.ПериодРегистрации = Дата(2026,1,1); Row.ВидРасчета = Salary;
   Row.Employee = Parts[0]; Row.Department = Parts[1]; Row.Amount = Число(Parts[2]);
   Row.БазовыйПериодНачало = Дата(2026,1,1); Row.БазовыйПериодКонец = КонецМесяца(Дата(2026,1,1));
  КонецЦикла;
  Set.Записать();
  Doc = Документы.Orders.СоздатьДокумент(); Doc.Номер = "BONUS"; Doc.Дата = Дата(2026,1,31); Doc.Записать();
  Set = РегистрыРасчета.Accruals.СоздатьНаборЗаписей(); Set.Отбор.Регистратор.Установить(Doc.Ссылка);
  Row = Set.Добавить(); Row.ПериодРегистрации = Дата(2026,1,1); Row.ВидРасчета = Bonus; Row.Employee = "Alice"; Row.Department = "A";
  Row.БазовыйПериодНачало = Дата(2026,1,1); Row.БазовыйПериодКонец = КонецМесяца(Дата(2026,1,1)); Set.Записать();
 КонецЕсли;
 Query = Новый Запрос;
 Query.Текст = "ВЫБРАТЬ Base.AmountБаза КАК Amount ИЗ РегистрРасчета.Accruals.БазаAccruals(&Dims, &Dims, , Регистратор = &Recorder) КАК Base";
 Query.УстановитьПараметр("Dims", СтрРазделить("Employee,Department", ","));
 Query.УстановитьПараметр("Recorder", Документы.Orders.НайтиПоНомеру("BONUS"));
 Selection = Query.Выполнить().Выбрать();
 Total = 0; Count = 0;
 Пока Selection.Следующий() Цикл Total = Total + Selection.Amount; Count = Count + 1; КонецЦикла;
 ReturnValue = "base=" + Формат(Total,"ЧГ=0; ЧДЦ=2") + ";rows=" + Строка(Count);
 ReturnValue = ReturnValue + ";flag=" + Строка(Метаданные.РегистрыРасчета.Accruals.Измерения.Employee.БазовоеИзмерение);
 Возврат ReturnValue;
КонецФункции
