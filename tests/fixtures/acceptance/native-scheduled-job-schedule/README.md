Raw `ScheduledJobs/KeyProbe/Ext/Schedule.xml` exports from disabled jobs:
XML 2.20 on 8.3.27.2342, 2.21 on 8.5.1.1529, 2.22 on 8.5.4.2028.

Each run used the corresponding `formats/platform_xml/<format>/all_kinds/src`
seed with the schedule from `2.20/staged_dump_roots/Schedule.xml`, adapted to
the line before import. The platform ran `ibcmd infobase create`,
`ibcmd config import`, `ibcmd config apply --force` and `ibcmd config export`.
Configuration.xml and the KeyProbe descriptor are the unchanged seed files.
The Schedule.xml files here are the exports, with their original BOM and CRLF.
