# Legacy inline registrations (#1370)

Synthetic configuration reproducing the owner layout written by Unica before
#1325: Form Main and Template Print appear as full descriptions in the catalog
and as matching standalone descriptors. It is not a platform export.

Command Open deliberately remains inline: this is its normal storage contract,
not the legacy Form/Template defect. The form, template and command have real
payload files so canonical owner check traverses the complete child footprints.
Tests also derive by-name registrations and an invalid CodeType control from
these same bytes. Any apply edit of the owner registers Form Main and
Template Print by name (S-1370-legacy-inline-collapse); no platform-import
result is claimed.
