
import requests
import hashlib
import uuid
import time
import sys

BASE_URL = "http://localhost:8080"

def log(msg):
    print(f"[TEST] {msg}")

def run_test():
    # 1. Register User
    email = f"test_audit_{uuid.uuid4()}@example.com"
    password = "password123"
    log(f"Registering user {email}...")
    
    try:
        resp = requests.post(f"{BASE_URL}/auth/register", json={
            "email": email,
            "password": password
        })
        resp.raise_for_status()
        data = resp.json()
        token = data["token"]
        log(f"Registered.")
    except Exception as e:
        log(f"Registration/Connection failed: {e}")
        sys.exit(1)

    headers = {"Authorization": f"Bearer {token}"}

    # 2. Upload Chunk/File (Legacy)
    content = b"Audit Verification Content"
    import base64
    b64_content = base64.b64encode(content).decode("utf-8")
    filename = f"audit_file_{uuid.uuid4()}.txt"
    
    log(f"Uploading file: {filename}")
    resp = requests.post(f"{BASE_URL}/files", json={
        "path": filename,
        "content": b64_content
    }, headers=headers)
    resp.raise_for_status()
    file_id = resp.json()["id"]
    log(f"Uploaded. File ID: {file_id}")
    
    # 3. Get Version ID
    resp = requests.get(f"{BASE_URL}/v1/files/{file_id}", headers=headers)
    resp.raise_for_status()
    version_id = resp.json()["current_version_id"]
    log(f"Got Version ID: {version_id}")
    
    # 4. Test Download (Version ID)
    log(f"Downloading by Version ID...")
    resp = requests.get(f"{BASE_URL}/v1/files/{version_id}/download", headers=headers)
    if resp.status_code == 200 and resp.content == content:
        log("SUCCESS: Version ID download valid.")
    else:
        log(f"FAILURE: Version ID download: {resp.status_code}")
        print(resp.text)
        sys.exit(1)

    # 5. Test Download (File ID)
    log(f"Downloading by File ID...")
    resp = requests.get(f"{BASE_URL}/v1/files/{file_id}/download", headers=headers)
    if resp.status_code == 200 and resp.content == content:
        log("SUCCESS: File ID download valid.")
    else:
        log(f"FAILURE: File ID download: {resp.status_code}")
        sys.exit(1)

if __name__ == "__main__":
    run_test()
