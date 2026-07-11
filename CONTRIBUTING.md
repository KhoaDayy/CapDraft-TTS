# Đóng góp cho CapDraft TTS

Cảm ơn bạn muốn cải thiện dự án. Hãy giữ thay đổi nhỏ, có mục tiêu rõ ràng và không đưa project CapCut, token, `device.json` hoặc đường dẫn máy cá nhân vào commit.

## Quy trình

1. Fork repository và tạo branch từ nhánh mặc định.
2. Tạo virtual environment, cài `requirements.txt`.
3. Thực hiện thay đổi và bổ sung test cho hành vi mới.
4. Chạy `python -m unittest discover -s tests -v`.
5. Mở pull request, mô tả vấn đề, giải pháp và cách đã kiểm tra.

## Quy ước commit

Dùng thông điệp ngắn ở dạng mệnh lệnh, ví dụ:

- `feat: add voice search filter`
- `fix: restore draft after validation failure`
- `docs: clarify ffmpeg setup`

## Báo lỗi

Issue nên có phiên bản Python/Windows, bước tái hiện, kết quả mong đợi, kết quả thực tế và log đã loại bỏ dữ liệu nhạy cảm. Không tải lên `draft_content.json` thật nếu chưa xóa nội dung riêng tư.
