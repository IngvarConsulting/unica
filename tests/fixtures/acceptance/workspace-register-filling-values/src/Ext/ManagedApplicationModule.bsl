Процедура ПриНачалеРаботыСистемы()
 Если ПустаяСтрока(ПараметрЗапуска) Тогда Возврат; КонецЕсли;
 Попытка
  Input = Новый Структура("DimensionValue,ResourceValue,AttributeValue", "D-input", "R-input", "A-input");
  Form = ПолучитьФорму("РегистрСведений.Facts.ФормаЗаписи", Новый Структура("ЗначенияЗаполнения", Input), , "with-data");
  Result = "input=" + Form.Запись.DimensionValue + "," + Form.Запись.ResourceValue + "," + Form.Запись.AttributeValue;
  Empty = ПолучитьФорму("РегистрСведений.Facts.ФормаЗаписи", Новый Структура("ЗначенияЗаполнения", Новый Структура), , "without-data");
  Result = Result + ";missing=" + Empty.Запись.DimensionValue + "," + Empty.Запись.ResourceValue + "," + Empty.Запись.AttributeValue;
 Исключение
  Result = "FAIL: " + ОписаниеОшибки();
 КонецПопытки;
 Text = Новый ТекстовыйДокумент;
 Text.УстановитьТекст(Result);
 Text.Записать(ПараметрЗапуска);
 ЗавершитьРаботуСистемы(Ложь);
КонецПроцедуры
