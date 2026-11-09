# BinaryData profile fixture

`native-type.xml` is the exact raw Type fragment of a ChartOfAccounts attribute exported by pinned 1C 8.3.27.2074. It preserves length 32 and Variable qualifiers. The source and second export have the same Type shape. The evidence used an isolated diagnostic input with the QName changed from the production-rejected `xs:binary` to `xs:base64Binary`; it does not claim production writer acceptance. The manifest records raw source and fragment hashes/range. The fragment inherits MDClasses, v8 and xs namespaces from its native owner; tests add those declarations explicitly.

Git retains the fragment bytes through its exact `-text` attribute. Public binaryData arguments and the form `binary` shorthand remain unchanged.
