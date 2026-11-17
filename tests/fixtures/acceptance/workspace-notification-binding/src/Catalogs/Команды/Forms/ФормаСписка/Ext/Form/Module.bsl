&AtClient
Procedure Start()
    Step();
    Show("😀", New NotifyDescription("Step", ThisObject), New NotifyDescription("Step", ThisObject));
    Callback = Новый ОписаниеОповещения("Step", ЭтотОбъект);
    Computed = New NotifyDescription(GetName(), ThisObject);
    Dynamic = New NotifyDescription("Step", GetReceiver());
    Duplicated = New NotifyDescription("Duplicate", ThisObject);
    Shared = New NotifyDescription("Finish", Notifications);
#If Server Then
    Unreachable = New NotifyDescription("Finish", Notifications);
    Unreachable = New NotifyDescription("Step", ThisObject);
#EndIf
EndProcedure

&AtClient
Procedure Step(Parameters = Undefined) Export
    Callback = New NotifyDescription("Done", ThisObject);
EndProcedure

&AtClient
Procedure Done(Parameters = Undefined) Export
EndProcedure

&AtClient
Procedure ParameterShadow(Notifications)
    Callback = New NotifyDescription("Finish", Notifications);
EndProcedure

&AtClient
Procedure LocalShadow()
    If True Then
        Notifications = GetReceiver();
    EndIf;
    Callback = New NotifyDescription("Finish", Notifications);
EndProcedure

&AtServer
Procedure ServerOrigin()
    Callback = New NotifyDescription("Finish", Notifications);
EndProcedure

#If Client Then
&AtClient
Procedure Duplicate() Export
EndProcedure
#Else
&AtClient
Procedure Duplicate() Export
EndProcedure
#EndIf
