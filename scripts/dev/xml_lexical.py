#!/usr/bin/env python3
"""Small fail-closed helpers for XML lexical contract checks."""

from __future__ import annotations


class LexicalXmlError(ValueError):
    """The raw XML bytes cannot provide a trustworthy root start tag."""


_XML_SPACE = " \t\r\n"


def _skip_space(text: str, index: int) -> int:
    while index < len(text) and text[index] in _XML_SPACE:
        index += 1
    return index


def _root_start(text: str) -> int:
    index = 0
    while True:
        index = _skip_space(text, index)
        if text.startswith("<?", index):
            end = text.find("?>", index + 2)
            if end < 0:
                raise LexicalXmlError("unterminated XML processing instruction")
            index = end + 2
            continue
        if text.startswith("<!--", index):
            end = text.find("-->", index + 4)
            if end < 0:
                raise LexicalXmlError("unterminated XML comment")
            index = end + 3
            continue
        if text.startswith("<!DOCTYPE", index):
            cursor = index + len("<!DOCTYPE")
            quote = None
            subset_depth = 0
            while cursor < len(text):
                character = text[cursor]
                if quote is not None:
                    if character == quote:
                        quote = None
                elif character in {"'", '"'}:
                    quote = character
                elif character == "[":
                    subset_depth += 1
                elif character == "]" and subset_depth:
                    subset_depth -= 1
                elif character == ">" and subset_depth == 0:
                    index = cursor + 1
                    break
                cursor += 1
            else:
                raise LexicalXmlError("unterminated DOCTYPE declaration")
            continue
        if text.startswith("<!", index):
            raise LexicalXmlError("unsupported declaration before the document element")
        if index >= len(text) or text[index] != "<" or text.startswith("</", index):
            raise LexicalXmlError("document element start tag is absent")
        return index


def raw_root_attribute(payload: bytes, attribute_name: str) -> str | None:
    """Return the undecoded lexical value of one unqualified root attribute."""
    if (
        not isinstance(payload, bytes)
        or not isinstance(attribute_name, str)
        or not attribute_name
        or any(character in _XML_SPACE + "<>/='\"" for character in attribute_name)
    ):
        raise LexicalXmlError("invalid raw-root-attribute request")
    try:
        text = payload.decode("utf-8-sig")
    except UnicodeDecodeError as error:
        raise LexicalXmlError(f"corpus XML is not UTF-8: {error}") from error

    index = _root_start(text) + 1
    name_start = index
    while index < len(text) and text[index] not in _XML_SPACE + "/>":
        index += 1
    if index == name_start:
        raise LexicalXmlError("document element name is absent")

    while True:
        index = _skip_space(text, index)
        if text.startswith("/>", index) or (
            index < len(text) and text[index] == ">"
        ):
            return None
        if index >= len(text):
            raise LexicalXmlError("unterminated document element start tag")

        name_start = index
        while index < len(text) and text[index] not in _XML_SPACE + "=/>":
            index += 1
        if index == name_start:
            raise LexicalXmlError("invalid root attribute name")
        name = text[name_start:index]
        index = _skip_space(text, index)
        if index >= len(text) or text[index] != "=":
            raise LexicalXmlError(f"root attribute {name!r} has no equals sign")
        index = _skip_space(text, index + 1)
        if index >= len(text) or text[index] not in {"'", '"'}:
            raise LexicalXmlError(f"root attribute {name!r} is not quoted")
        quote = text[index]
        value_start = index + 1
        value_end = text.find(quote, value_start)
        if value_end < 0:
            raise LexicalXmlError(f"root attribute {name!r} is unterminated")
        if name == attribute_name:
            return text[value_start:value_end]
        index = value_end + 1
