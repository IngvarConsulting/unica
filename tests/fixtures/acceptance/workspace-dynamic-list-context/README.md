Синтетическая XML+BSL фикстура #1565 для 8.3.27: List привязывает OnGetDataAtServer к серверному обработчику без контекста, Generate оставляет событие доступным для event.implement. Форма OnCreateAtServer служит контекстным контролем.

Независимые основания:
- [Статья 1С о динамическом списке](https://wonderland.v8.1c.ru/blog/obrabotka-i-oformlenie-dannykh-dinamicheskogo-spiska/) от 08.07.2016, платформа 8.3.10.2168: обработчик работает без контекста формы. [Пример](https://wonderland.v8.1c.ru/upload/medialibrary/736/736dad82bd8a330bd8269db6e181192b.png) показывает директиву и три параметра.
- [Практическое руководство 1С, обработчики формы](https://kb.1ci.com/1C_Enterprise_Platform/Tutorials/Practical_developer_guide_8.3/Lesson_5._Theory/Event_handler_procedures_in_form_modules/), пример 5.18: OnCreateAtServer с AtServer и двумя параметрами.
- [Пояснение 1С о реквизитах и элементах формы](https://kb.1ci.com/1C_Enterprise_Platform/FAQ/Development/Controls/Attributes_and_items_of_the_form__referring_to_them_from_the_module_of_the_form/?language=en): в OnCreateAtServer доступен контекст формы.

Пример вендора подтверждает допустимость NoContext, но не доказывает запрет прежнего AtServer для OnGetDataAtServer. Прежняя классификация AtServer сохраняется только для этого события; генератор выбирает NoContext.
