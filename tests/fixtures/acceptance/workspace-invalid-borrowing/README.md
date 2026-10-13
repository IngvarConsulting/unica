# Malformed borrowing identity

S411 is a synthetic negative reader fixture derived from
`../workspace-incomplete-borrowed/`. Configuration registrations are reduced to
the language and common module. The module descriptor has a second
`ExtendedConfigurationObject` containing `broken` after the valid parent UUID.
The BSL and source-set declaration are synthetic inputs from S410.

This deliberately damaged workspace is not evidence of valid platform XML.
Module and method reads must report `source_unreadable`; a valid first UUID
must not hide the duplicate. SHA-256 assertions verify that reads leave all
source files unchanged. S410 covers successful reads of the undamaged input.
