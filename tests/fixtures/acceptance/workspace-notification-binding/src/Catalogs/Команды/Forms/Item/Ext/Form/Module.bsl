&AtClient
Procedure Start()
    Callback = New NotifyDescription("Finish", Notifications);
    Callback = New NotifyDescription("Step", ThisObject);
EndProcedure

&AtClient
Procedure Step(Parameters = Undefined) Export
EndProcedure
