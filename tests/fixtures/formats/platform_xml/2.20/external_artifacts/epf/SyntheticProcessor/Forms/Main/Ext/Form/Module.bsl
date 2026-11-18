&НаСервере
Процедура OnCreateAtServer(Отказ, СтандартнаяОбработка)
	Counter = 1;
КонецПроцедуры

&НаКлиенте
Процедура TextValueOnChange(Элемент)
	Counter = Counter + 1;
КонецПроцедуры

&НаКлиенте
Процедура Refresh(Команда)
	RefreshAtServer();
КонецПроцедуры

&НаСервере
Процедура RefreshAtServer()
	Counter = 0;
КонецПроцедуры
