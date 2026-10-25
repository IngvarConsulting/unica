Procedure RunJob() Export
EndProcedure

Procedure OnBeforeWrite(Source, Cancel) Export
	Selection = Catalogs.ПримерВ.Select();
EndProcedure
