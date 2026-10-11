FUNCTION ZPYRFCX_MEM_ROWS.
*"----------------------------------------------------------------------
*"*"Local Interface:
*"  IMPORTING
*"     VALUE(IV_ROWS) TYPE  I
*"     VALUE(IV_FILL) TYPE  I DEFAULT 15
*"  TABLES
*"      ET_ROWS STRUCTURE  ZPYRFCX_MEM_ROW
*"----------------------------------------------------------------------
* Read-only data source for client memory benchmarks: every field
* value is unique per row and per field. No database access.
  DATA lv_fill  TYPE i.
  DATA lv_row   TYPE i.
  DATA lv_field TYPE i.
  DATA lv_value TYPE c LENGTH 15.
  DATA ls_row   TYPE zpyrfcx_mem_row.
  FIELD-SYMBOLS <lv_comp> TYPE any.

  CLEAR et_rows[].

  lv_fill = iv_fill.
  IF lv_fill < 0.
    lv_fill = 0.
  ELSEIF lv_fill > 15.
    lv_fill = 15.
  ENDIF.

  DO iv_rows TIMES.
    lv_row = sy-index.
    CLEAR ls_row.
    IF lv_fill > 0.
      DO 40 TIMES.
        lv_field = sy-index.
        ASSIGN COMPONENT lv_field OF STRUCTURE ls_row TO <lv_comp>.
        lv_value = |R{ lv_row WIDTH = 7 ALIGN = RIGHT PAD = '0' }F{ lv_field WIDTH = 2 ALIGN = RIGHT PAD = '0' }XXXX|.
        <lv_comp> = lv_value(lv_fill).
      ENDDO.
    ENDIF.
    APPEND ls_row TO et_rows.
  ENDDO.
ENDFUNCTION.
