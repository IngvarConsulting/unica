Procedure Recount(Mode) Export
	Var Total;
	For Each Line In ThisObject.Lines Do
		Total = Tools.Add(Line.Quantity, );
	EndDo;
	Message(Total);
EndProcedure
