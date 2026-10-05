#!/usr/bin/env python3
import asyncio
import json
import os
import sys
import argparse
import signal

class ComputeServer:
    def __init__(self):
        self.methods = {
            "ping": self.handle_ping,
            "noise_reduction": self.handle_noise_reduction,
            "transcribe": self.handle_transcribe,
        }

    async def handle_ping(self, params):
        return {"status": "ok", "version": "1.0.0"}

    async def handle_noise_reduction(self, params):
        try:
            from df.enhance import enhance, init_df, load_audio, save_audio
            model, df_state, _ = init_df()
            audio, _ = load_audio(params["source_path"], sr=df_state.sr())
            enhanced = enhance(model, df_state, audio)
            save_audio(params["output_path"], enhanced, df_state.sr())
            return {"success": True, "output_path": params["output_path"]}
        except ImportError:
            # Fallback if optional ML library is not installed
            source = params.get("source_path", "")
            target = params.get("output_path", "")
            if source and target and os.path.exists(source):
                import shutil
                shutil.copyfile(source, target)
            return {"success": True, "output_path": target, "warning": "deepfilternet not installed; used passthrough"}

    async def handle_transcribe(self, params):
        return {"success": True, "text": "whisper stub transcription"}

    async def handle_connection(self, reader, writer):
        while True:
            line = await reader.readline()
            if not line:
                break
            try:
                request = json.loads(line.decode('utf-8'))
            except Exception:
                continue

            req_id = request.get("id")
            method_name = request.get("method")
            method = self.methods.get(method_name)

            if method:
                try:
                    result = await method(request.get("params", {}))
                    response = {"jsonrpc": "2.0", "id": req_id, "result": result}
                except Exception as e:
                    response = {
                        "jsonrpc": "2.0",
                        "id": req_id,
                        "error": {"code": -32000, "message": str(e)},
                    }
            else:
                response = {
                    "jsonrpc": "2.0",
                    "id": req_id,
                    "error": {"code": -32601, "message": f"Method '{method_name}' not found"},
                }
            writer.write(json.dumps(response).encode('utf-8') + b"\n")
            await writer.drain()

    async def run(self, socket_path):
        if os.path.exists(socket_path):
            try:
                os.remove(socket_path)
            except OSError:
                pass
        server = await asyncio.start_unix_server(self.handle_connection, path=socket_path)
        try:
            async with server:
                await server.serve_forever()
        finally:
            if os.path.exists(socket_path):
                try:
                    os.remove(socket_path)
                except OSError:
                    pass

def main():
    parser = argparse.ArgumentParser(description="Tempo Compute Server")
    parser.add_argument("--socket", required=True, help="Path to Unix domain socket")
    args = parser.parse_args()

    loop = asyncio.new_event_loop()
    asyncio.set_event_loop(loop)

    server = ComputeServer()
    try:
        loop.run_until_complete(server.run(args.socket))
    except (KeyboardInterrupt, SystemExit):
        pass
    finally:
        loop.close()

if __name__ == "__main__":
    main()
