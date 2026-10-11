# ABAP test objects

The live test suites call two custom function modules. Create these objects on the test system before you run them.

- `ZPYRFCX_TEST` (function group): holds the two function modules.
- `ZPYRFCX_FE_DATATYPES` (function module): echoes its input; it copies `IS_INPUT` to `ES_OUTPUT`.
- `ZPYRFCX_MEM_ROWS` (function module): returns `IV_ROWS` rows in table `ET_ROWS`, for memory benchmarks.
- `ZPYRFCX_FE_INPUT` (structure): one field per data type under test.
- `ZPYRFCX_MEM_ROW` (structure): one row of 40 `CHAR(15)` fields.

Both function modules must be remote-enabled (processing type "Remote-Enabled Module").

Release 750 cannot have a `UTCLONG` field. On release 757 and later, `ZPYRFCX_FE_INPUT` has one more field, `ZUTCLONG`. Use the files in `release-757/` there; they replace the files of the same name in this folder.

The files follow the abapGit format: `*.fugr.xml` holds the function group and the function module interfaces, `*.fugr.*.abap` holds the source of each include and function module, and `*.tabl.xml` holds a structure. Import them with abapGit into a package of your choice.
