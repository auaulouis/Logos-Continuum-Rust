import urllib.request, sys, time
def run():
    with open("tmp/bench-compare/source_docs/Mega LD Backfile Part 1 .docx", "rb") as f:
        file_bytes = f.read()
    boundary = f'----logosBoundary{int(time.time() * 1000)}'
    chunks = []
    def add(text: str): chunks.append(text.encode('utf-8'))
    add(f'--{boundary}\r\n')
    add(f'Content-Disposition: form-data; name="file"; filename="Mega.docx"\r\n')
    add('Content-Type: application/vnd.openxmlformats-officedocument.wordprocessingml.document\r\n\r\n')
    chunks.append(file_bytes)
    add('\r\n')
    add(f'--{boundary}--\r\n')
    body = b''.join(chunks)

    req = urllib.request.Request(
        "http://127.0.0.1:5002/upload-docx",
        data=body,
        headers={"Content-Type": f"multipart/form-data; boundary={boundary}"},
        method="POST"
    )
    urllib.request.urlopen(req)
run()
