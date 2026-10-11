FUNCTION ZPYRFCX_FE_DATATYPES.
*"----------------------------------------------------------------------
*"*"Local Interface:
*"  IMPORTING
*"     VALUE(IS_INPUT) TYPE  ZPYRFCX_FE_INPUT OPTIONAL
*"     VALUE(IV_COUNT) TYPE  I DEFAULT 0
*"  EXPORTING
*"     VALUE(ES_OUTPUT) TYPE  ZPYRFCX_FE_INPUT
*"----------------------------------------------------------------------
  es_output = is_input.
ENDFUNCTION.
