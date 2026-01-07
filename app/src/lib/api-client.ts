import { AdminService } from "@/gen/admin_connect";
import { WorkflowService } from "@/gen/workflow_connect";
import { WorkflowScheduleService } from "@/gen/workflow_schedule_connect";
import { createClient } from "@connectrpc/connect";
import { createGrpcWebTransport } from "@connectrpc/connect-web";

const API_BASE_URL = import.meta.env.VITE_KAGZI_API_URL || "http://localhost:50051";

const transport = createGrpcWebTransport({
  baseUrl: API_BASE_URL,
});

export const adminClient = createClient(AdminService, transport);
export const workflowClient = createClient(WorkflowService, transport);
export const scheduleClient = createClient(WorkflowScheduleService, transport);
export { transport };
