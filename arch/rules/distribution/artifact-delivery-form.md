---
id: INV.PKG.ARTIFACT-DELIVERY-FORM
check:
  - crates/unica-bootstrap/tests/manifest_contract.rs::an_artifact_delivered_as_one_file_is_accepted
  - crates/unica-bootstrap/tests/manifest_contract.rs::a_one_file_artifact_declares_exactly_the_file_it_delivers
  - crates/unica-bootstrap/tests/manifest_contract.rs::the_core_is_still_required_to_arrive_as_an_archive
  - crates/unica-bootstrap/tests/runtime_install.rs::an_artifact_delivered_as_one_file_is_installed_without_unpacking
  - crates/unica-bootstrap/tests/runtime_install.rs::a_file_artifact_whose_bytes_are_wrong_is_refused_like_any_other
---

# Движок может поставляться одним файлом без архива

Ядро Unica поставляется архивом. Движок может поставляться также одиночным
файлом с типом `application/octet-stream`; его манифест объявляет ровно один
файл установки.

Bootstrap сохраняет такие байты без распаковки. Несовпадение SHA-256
останавливает установку, как и при [получении архива](archive-checksum-before-unpack.md).
