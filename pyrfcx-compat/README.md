# pyrfcx-compat

`pyrfcx-compat` is a drop-in replacement for the client part of SAP's pyrfc
3.4. It installs an import package named `pyrfc` that runs on
pyrfcx. Existing code runs unchanged.

The facade reproduces pyrfc 3.4 client behaviour, bugs included. The project
measures this behaviour against real pyrfc 3.4.

```powershell
pip install pyrfcx-compat
```

```python
from pyrfc import Connection

with Connection(dest="MY_DEST") as conn:
    print(conn.call("STFC_CONNECTION", REQUTEXT="hi")["ECHOTEXT"])
```

You need the SAP NW RFC SDK 7.50 and `SAPNWRFC_HOME`, as for pyrfcx. The
SDK is SAP property and is not included.

Not supported: server RFC, tRFC, qRFC, bgRFC, and the throughput API.

## Differences from pyrfc 3.3.1

The facade follows pyrfc 3.4. If you move from pyrfc 3.3.1, expect these
differences:

- The message of a `CommunicationError` has no trailing newline. pyrfc
  3.3.1 keeps the newline that the SDK sends. pyrfc 3.4 right-strips the
  message, and the facade does the same. Code that stores the message
  stores one character less.

The list of pyrfc 3.4 bugs that this package keeps, and the full
documentation, are in the pyrfcx repository:
https://github.com/bougebouge/pyrfcx
