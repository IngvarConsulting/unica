#Region Public

// Adds two numbers.
// The second one is optional.
Function Add(Val First, Second = 1) Export
	Result = First + Second;
	If Result > 0 Then
		Return Result;
	EndIf;
	Return 0;
EndFunction

Procedure Hidden()
EndProcedure

#EndRegion
