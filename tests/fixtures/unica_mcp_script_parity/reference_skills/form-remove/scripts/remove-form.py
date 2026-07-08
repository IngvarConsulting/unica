#!/usr/bin/env python3
# remove-form v1.3 — Remove form from 1C object
# Source: https://github.com/Nikolay-Shirokov/cc-1c-skills

import argparse
import os
import re
import shutil
import sys

from lxml import etree

NSMAP = {"md": "http://v8.1c.ru/8.3/MDClasses"}


def local_name(node):
    return etree.QName(node).localname if isinstance(node.tag, str) else ""


def collapse_empty_touched_elements(root):
    """Unica parity patch: keep touched empty form slots and ChildObjects self-closing."""
    for el in root.iter():
        name = local_name(el)
        if name != "ChildObjects" and not name.endswith("Form"):
            continue
        if len(el) == 0 and (el.text is None or el.text.strip() == ""):
            el.text = None


def save_xml_with_bom(tree, path):
    """Save XML tree to file with UTF-8 BOM."""
    xml_bytes = etree.tostring(tree, xml_declaration=True, encoding="UTF-8")
    xml_bytes = xml_bytes.replace(b"<?xml version='1.0' encoding='UTF-8'?>", b'<?xml version="1.0" encoding="utf-8"?>')
    if not xml_bytes.endswith(b"\n"):
        xml_bytes += b"\n"
    with open(path, "wb") as f:
        f.write(b"\xef\xbb\xbf")
        f.write(xml_bytes)


def main():
    sys.stdout.reconfigure(encoding="utf-8")
    sys.stderr.reconfigure(encoding="utf-8")
    parser = argparse.ArgumentParser(description="Remove form from 1C object", allow_abbrev=False)
    parser.add_argument("-ObjectName", "-ProcessorName", required=True)
    parser.add_argument("-FormName", required=True)
    parser.add_argument("-SrcDir", default="src")
    args = parser.parse_args()

    object_name = args.ObjectName
    form_name = args.FormName
    src_dir = args.SrcDir

    # --- Checks ---

    root_xml_path = os.path.join(src_dir, f"{object_name}.xml")
    if not os.path.exists(root_xml_path):
        print(f"Корневой файл обработки не найден: {root_xml_path}", file=sys.stderr)
        sys.exit(1)

    processor_dir = os.path.join(src_dir, object_name)
    forms_dir = os.path.join(processor_dir, "Forms")
    form_meta_path = os.path.join(forms_dir, f"{form_name}.xml")
    form_dir = os.path.join(forms_dir, form_name)

    if not os.path.exists(form_meta_path):
        print(f"Метаданные формы не найдены: {form_meta_path}", file=sys.stderr)
        sys.exit(1)

    # --- Delete files ---

    if os.path.isdir(form_dir):
        shutil.rmtree(form_dir)
        print(f"[OK] Удалён каталог: {form_dir}")

    os.remove(form_meta_path)
    print(f"[OK] Удалён файл: {form_meta_path}")

    # --- Modify root XML ---

    root_xml_full = os.path.abspath(root_xml_path)
    parser_xml = etree.XMLParser(remove_blank_text=False)
    tree = etree.parse(root_xml_full, parser_xml)
    root = tree.getroot()

    # Remove <Form>FormName</Form> from ChildObjects
    for node in root.findall(".//md:ChildObjects/md:Form", NSMAP):
        if node.text and node.text.strip() == form_name:
            parent = node.getparent()
            prev = node.getprevious()
            if prev is not None:
                # Preserve whitespace that separated the removed node from the next sibling/close tag.
                if prev.tail and prev.tail.strip() == "":
                    prev.tail = node.tail if node.tail and node.tail.strip() == "" else prev.tail
            else:
                # First child — parent.text separates ChildObjects from the next sibling/close tag.
                if parent.text and parent.text.strip() == "":
                    parent.text = node.tail if node.tail and node.tail.strip() == "" else parent.text
            parent.remove(node)
            break

    # Clear any Default*/Auxiliary* form slot that pointed to the removed form
    # (form-add writes the purpose-specific property: DefaultObjectForm / DefaultListForm /
    #  DefaultChoiceForm / DefaultRecordForm / DefaultForm — not just generic DefaultForm).
    ref_re = re.compile(rf"Form\.{re.escape(form_name)}$")
    for el in root.iter():
        if not isinstance(el.tag, str):
            continue
        if etree.QName(el).localname.endswith("Form") and el.text and ref_re.search(el.text):
            el.text = None

    collapse_empty_touched_elements(root)

    # Save with BOM
    save_xml_with_bom(tree, root_xml_full)

    print(f"[OK] Форма {form_name} удалена из {root_xml_path}")


if __name__ == "__main__":
    main()
